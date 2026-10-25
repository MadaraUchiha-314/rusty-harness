//! Driving a real terminal: crossterm events in, A2A requests out (R27.3).

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    Event, EventStream, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use futures::StreamExt as _;
use tokio::sync::mpsc;

use super::app::{App, Incoming, Outgoing};
use super::view;
use crate::harness::a2a_client::official::{OfficialTransport, fetch_card};
use crate::harness::a2a_client::{AgentTransport, CardError};
use crate::host::Host;

/// Why the TUI stopped.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// The harness's card could not be fetched or used.
    #[error("{0}")]
    Card(#[from] CardError),
    /// The terminal refused.
    #[error("terminal: {0}")]
    Terminal(#[from] std::io::Error),
}

/// Runs `app` against the harness at `url` until the person leaves.
///
/// # Errors
///
/// When the card cannot be fetched or the terminal cannot be set up.
pub async fn run(mut app: App, url: &url::Url, host: &Host) -> Result<(), TuiError> {
    let card = fetch_card(host, url).await?;
    let extensions: Vec<String> = card
        .capabilities
        .extensions
        .iter()
        .flatten()
        .map(|e| e.uri.clone())
        .collect();
    let transport: Arc<OfficialTransport> =
        Arc::new(OfficialTransport::from_card(&card, None, extensions)?);

    let mut terminal = ratatui::try_init()?;
    let enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
    }
    app.set_picker(ratatui_image::picker::Picker::from_query_stdio().ok());

    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let result = loop {
        if let Err(e) = terminal.draw(|frame| view::draw(&mut app, frame)) {
            break Err(TuiError::Terminal(e));
        }
        if app.quit {
            break Ok(());
        }
        tokio::select! {
            event = events.next() => {
                match event {
                    Some(Ok(Event::Key(key))) => {
                        let now = host.clock().map_or_else(|_| chrono::Utc::now(), |c| c.now());
                        for outgoing in app.key(key, now) {
                            dispatch(outgoing, &transport, &tx);
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Err(TuiError::Terminal(e)),
                    None => break Ok(()),
                }
            }
            incoming = rx.recv() => {
                if let Some(incoming) = incoming {
                    let now = host.clock().map_or_else(|_| chrono::Utc::now(), |c| c.now());
                    app.receive(incoming, now);
                }
            }
            _ = tick.tick() => {}
        }
    };
    if enhanced {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

/// Sends one request on its own task; every event or error comes back through `tx`.
fn dispatch(
    outgoing: Outgoing,
    transport: &Arc<OfficialTransport>,
    tx: &mpsc::UnboundedSender<Incoming>,
) {
    let transport = Arc::clone(transport);
    let tx = tx.clone();
    match outgoing {
        Outgoing::Quit => {}
        Outgoing::Cancel(task) => {
            tokio::spawn(async move {
                let request = a2a::CancelTaskRequest {
                    id: task,
                    metadata: None,
                    tenant: None,
                };
                match transport.cancel_task(request).await {
                    Ok(task) => {
                        let _ = tx.send(Incoming::Response(Box::new(a2a::StreamResponse::Task(
                            task,
                        ))));
                    }
                    Err(e) => {
                        let _ = tx.send(Incoming::Error(e.to_string()));
                    }
                }
            });
        }
        Outgoing::Send(message) => {
            tokio::spawn(async move {
                let request = a2a::SendMessageRequest {
                    message: *message,
                    configuration: None,
                    metadata: None,
                    tenant: None,
                };
                match transport.send_streaming_message(request, Vec::new()).await {
                    Ok(mut stream) => {
                        while let Some(item) = stream.next().await {
                            let _ = match item {
                                Ok(response) => tx.send(Incoming::Response(Box::new(response))),
                                Err(e) => tx.send(Incoming::Error(e.to_string())),
                            };
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Incoming::Error(e.to_string()));
                    }
                }
            });
        }
    }
}
