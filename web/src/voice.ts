// Voice in the browser (R26.4, R28.6): the voice button records the microphone, the
// recording is decoded and resampled to 16-bit PCM at 24 kHz mono (what the harness
// speaks), and sent as one `voice/v1` audio part; spoken replies (audio artifacts) are
// played with an AudioContext. The provider session and its key stay on the harness.

export const VOICE_EXT = "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/voice/v1";
export const PCM_MEDIA_TYPE = "audio/pcm";
export const RATE = 24_000;

/** Samples in [-1, 1] as little-endian 16-bit PCM (clipped). */
export function floatToPcm16(samples: Float32Array): Uint8Array {
  const out = new Uint8Array(samples.length * 2);
  const view = new DataView(out.buffer);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i] ?? 0));
    view.setInt16(i * 2, Math.round(s * 32_767), true);
  }
  return out;
}

/** Little-endian 16-bit PCM as samples in [-1, 1]. */
export function pcm16ToFloat(bytes: Uint8Array): Float32Array {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const out = new Float32Array(Math.floor(bytes.byteLength / 2));
  for (let i = 0; i < out.length; i++) out[i] = view.getInt16(i * 2, true) / 32_768;
  return out;
}

/** Bytes as base64, for a raw part in ProtoJSON. */
export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

/** The `voice/v1` audio part for one utterance. */
export function audioPartJson(pcm: Uint8Array, session: string): Record<string, unknown> {
  return {
    raw: toBase64(pcm),
    mediaType: PCM_MEDIA_TYPE,
    metadata: {
      [VOICE_EXT]: {
        seq: 1,
        attempt: { iteration: 0, n: 1 },
        format: "pcm16@24k",
        final: true,
        session,
      },
    },
  };
}

/** A recording decoded and resampled to 24 kHz mono PCM16. */
export async function toPcm24k(recording: Blob): Promise<Uint8Array> {
  const encoded = await recording.arrayBuffer();
  const decoder = new AudioContext();
  try {
    const decoded = await decoder.decodeAudioData(encoded);
    const frames = Math.max(1, Math.ceil(decoded.duration * RATE));
    const offline = new OfflineAudioContext(1, frames, RATE);
    const source = offline.createBufferSource();
    source.buffer = decoded;
    source.connect(offline.destination);
    source.start();
    const rendered = await offline.startRendering();
    return floatToPcm16(rendered.getChannelData(0));
  } finally {
    void decoder.close();
  }
}

/** A capture in progress. */
export interface Recording {
  stop(): Promise<Uint8Array>;
  cancel(): void;
}

/** Starts recording the default microphone. */
export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const recorder = new MediaRecorder(stream);
  const chunks: Blob[] = [];
  recorder.addEventListener("dataavailable", (e) => chunks.push(e.data));
  recorder.start();
  const release = () => {
    for (const track of stream.getTracks()) track.stop();
  };
  return {
    stop: () =>
      new Promise<Uint8Array>((resolve, reject) => {
        recorder.addEventListener(
          "stop",
          () => {
            release();
            toPcm24k(new Blob(chunks, { type: recorder.mimeType })).then(resolve, reject);
          },
          { once: true },
        );
        recorder.stop();
      }),
    cancel: () => {
      recorder.stop();
      release();
    },
  };
}

/** Plays 24 kHz mono PCM16. */
export async function play(pcm: Uint8Array): Promise<void> {
  const samples = pcm16ToFloat(pcm);
  if (samples.length === 0) return;
  const context = new AudioContext({ sampleRate: RATE });
  const buffer = context.createBuffer(1, samples.length, RATE);
  buffer.getChannelData(0).set(samples);
  const source = context.createBufferSource();
  source.buffer = buffer;
  source.connect(context.destination);
  await new Promise<void>((resolve) => {
    source.addEventListener("ended", () => resolve(), { once: true });
    source.start();
  });
  await context.close();
}
