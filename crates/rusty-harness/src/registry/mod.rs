//! Core entities and the registry: `Entity`, `EntityId`, `EntityKind`, `Placement`,
//! `Registry` (R1).
//!
//! Every pluggable unit of the harness — a prompt, skill, tool, model, hook, plugin,
//! command, agent, channel, surface, renderer, store or durable backend — is an entity
//! with one identity `(id, version)` and one trait per kind. The registry holds one typed
//! map per kind, refuses a second registration of the same identity unless the caller
//! asks to replace it, and resolves an identity to the same instance whether it is local
//! or a proxy to something remote: callers never learn which.

use core::any::{Any, TypeId};
use core::fmt;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::runtime::{MaybeSend, MaybeSync};

/// The identity of an entity instance: its id and, for versioned kinds, its version.
///
/// `version` is `None` for the kinds the design declares unversioned (surfaces,
/// renderers, channels, built-in hooks and commands) and for a registration that did not
/// state one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId {
    /// The name the entity is registered under, unique within its kind.
    pub id: Arc<str>,
    /// The version, when the kind is versioned.
    pub version: Option<semver::Version>,
}

impl EntityId {
    /// An unversioned identity.
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            version: None,
        }
    }

    /// A versioned identity.
    #[must_use]
    pub fn versioned(id: impl Into<Arc<str>>, version: semver::Version) -> Self {
        Self {
            id: id.into(),
            version: Some(version),
        }
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(v) => write!(f, "{}@{v}", self.id),
            None => f.write_str(&self.id),
        }
    }
}

/// The kinds of entity the harness registers (R1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum EntityKind {
    /// A prompt fragment (R33).
    Prompt,
    /// An Agent Skills skill (R6).
    Skill,
    /// A tool, local or from an MCP server (R8).
    Tool,
    /// A language model (R9).
    Llm,
    /// A System One model (R9.4).
    SystemOne,
    /// A hook (R4).
    Hook,
    /// An agent plugin (R5).
    Plugin,
    /// A slash command (R10).
    Command,
    /// An agent: the harness's own, a remote one, or a loopback (R11, R17).
    Agent,
    /// A channel (R18).
    Channel,
    /// A surface (R25).
    Surface,
    /// A renderer (R25).
    Renderer,
    /// A persistence store (R14).
    PersistenceStore,
    /// A durable-execution backend (R22).
    DurableExecution,
    /// A speech-to-text provider (R28).
    SpeechToText,
    /// A text-to-speech provider (R28).
    TextToSpeech,
    /// A speech-to-speech voice provider (R28).
    Voice,
}

/// Where an entity runs (R1.3–4). Callers resolve by identity and never need to know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// In this process.
    Local,
    /// Behind a declared application protocol; the registered instance is a proxy.
    Remote(RemoteEndpoint),
}

/// The protocol and address a remote entity is reached through (R1.4).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteEndpoint {
    /// An agent, by its A2A agent card.
    A2a {
        /// Where the card is served.
        card_url: url::Url,
    },
    /// A tool, by the MCP server that hosts it.
    Mcp {
        /// The registered MCP server.
        server: EntityId,
    },
    /// A model or hook, by a provider's HTTP API.
    Http {
        /// The API's base URL.
        base_url: url::Url,
    },
}

/// Who registered an entity; names both parties when two collide (R5.6).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Provenance {
    /// Shipped with the crate.
    BuiltIn,
    /// Contributed by a plugin.
    Plugin(EntityId),
    /// Registered programmatically by the embedding application.
    Sdk,
    /// Declared in configuration.
    Config,
}

/// What every entity kind's trait extends (R1.1).
///
/// The trait is dyn-compatible on purpose: every kind is stored and resolved as
/// `Arc<dyn Kind>`, so no associated constants or generic methods.
pub trait Entity: MaybeSend + MaybeSync + 'static {
    /// The kind this instance belongs to.
    fn kind(&self) -> EntityKind;
    /// The identity this instance is registered under.
    fn entity_id(&self) -> &EntityId;
    /// Where the instance runs; local unless it is a proxy.
    fn placement(&self) -> Placement {
        Placement::Local
    }
}

/// A registry failure (R1.6): never a silent fallback to another entity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// No entity of that kind has that identity.
    #[error("no {kind:?} registered as {entry}")]
    NotFound {
        /// The kind looked up.
        kind: EntityKind,
        /// The identity looked up.
        entry: EntityId,
    },
    /// That identity is already taken for that kind, by `holder`.
    #[error("{kind:?} {entry} is already registered by {holder:?}")]
    Conflict {
        /// The kind registered.
        kind: EntityKind,
        /// The identity that collided.
        entry: EntityId,
        /// Who holds it.
        holder: Provenance,
    },
}

#[cfg(not(target_arch = "wasm32"))]
type AnyBox = Box<dyn Any + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type AnyBox = Box<dyn Any>;

struct Slot {
    kind: EntityKind,
    instance: AnyBox,
    provenance: Provenance,
}

/// One typed map per entity type, keyed by the trait object the kind is resolved as.
#[derive(Default)]
pub struct Registry {
    slots: RwLock<BTreeMap<TypeId, BTreeMap<EntityId, Slot>>>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let maps = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("Registry")
            .field("entities", &maps.values().map(BTreeMap::len).sum::<usize>())
            .finish()
    }
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `entity` as provided by the SDK; see [`Registry::register_with`].
    ///
    /// # Errors
    ///
    /// [`RegistryError::Conflict`] when the identity is taken for this type.
    pub fn register<E: Entity + ?Sized>(&self, entity: Arc<E>) -> Result<(), RegistryError> {
        self.register_with(entity, Provenance::Sdk)
    }

    /// Registers `entity` under its own identity, recording who contributed it (R1.2).
    ///
    /// # Errors
    ///
    /// [`RegistryError::Conflict`] when the identity is taken for this type; the error
    /// names the holder so a plugin collision reports both parties (R5.6).
    pub fn register_with<E: Entity + ?Sized>(
        &self,
        entity: Arc<E>,
        provenance: Provenance,
    ) -> Result<(), RegistryError> {
        let kind = entity.kind();
        let entry = entity.entity_id().clone();
        let mut maps = self
            .slots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = maps.entry(TypeId::of::<E>()).or_default();
        if let Some(holder) = map.get(&entry) {
            return Err(RegistryError::Conflict {
                kind,
                entry,
                holder: holder.provenance.clone(),
            });
        }
        map.insert(
            entry,
            Slot {
                kind,
                instance: Box::new(entity),
                provenance,
            },
        );
        Ok(())
    }

    /// Checks that `entry` is free for type `E`, naming the holder when it is not (what
    /// an all-or-nothing installer asks before registering anything).
    ///
    /// # Errors
    ///
    /// [`RegistryError::Conflict`] when the identity is taken.
    pub fn ensure_free<E: Entity + ?Sized>(
        &self,
        entry: &EntityId,
        kind: EntityKind,
    ) -> Result<(), RegistryError> {
        let maps = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match maps.get(&TypeId::of::<E>()).and_then(|map| map.get(entry)) {
            Some(slot) => Err(RegistryError::Conflict {
                kind,
                entry: entry.clone(),
                holder: slot.provenance.clone(),
            }),
            None => Ok(()),
        }
    }

    /// Replaces whatever holds `entity`'s identity, returning the previous instance
    /// (R1.5): the explicit way to swap a default for one's own.
    pub fn replace<E: Entity + ?Sized>(&self, entity: Arc<E>) -> Option<Arc<E>> {
        let entry = entity.entity_id().clone();
        let kind = entity.kind();
        let mut maps = self
            .slots
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = maps.entry(TypeId::of::<E>()).or_default();
        let previous = map.insert(
            entry,
            Slot {
                kind,
                instance: Box::new(entity),
                provenance: Provenance::Sdk,
            },
        );
        previous.and_then(|slot| slot.instance.downcast::<Arc<E>>().ok().map(|boxed| *boxed))
    }

    /// Resolves `entry` to the registered instance (R1.3).
    ///
    /// # Errors
    ///
    /// [`RegistryError::NotFound`] naming the kind and the entry; never a different
    /// entity (R1.6).
    pub fn get<E: Entity + ?Sized>(&self, entry: &EntityId) -> Result<Arc<E>, RegistryError> {
        let maps = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        maps.get(&TypeId::of::<E>())
            .and_then(|map| map.get(entry))
            .and_then(|slot| slot.instance.downcast_ref::<Arc<E>>())
            .cloned()
            .ok_or_else(|| RegistryError::NotFound {
                kind: kind_of::<E>(&maps),
                entry: entry.clone(),
            })
    }

    /// Every registered instance of this type, in identity order.
    #[must_use]
    pub fn list<E: Entity + ?Sized>(&self) -> Vec<Arc<E>> {
        let maps = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        maps.get(&TypeId::of::<E>())
            .map(|map| {
                map.values()
                    .filter_map(|slot| slot.instance.downcast_ref::<Arc<E>>())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The highest registered version of `id` for this type, or the unversioned entry.
    ///
    /// # Errors
    ///
    /// [`RegistryError::NotFound`] when nothing is registered under `id`.
    pub fn latest<E: Entity + ?Sized>(&self, id: &str) -> Result<Arc<E>, RegistryError> {
        let maps = self
            .slots
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        maps.get(&TypeId::of::<E>())
            .and_then(|map| {
                map.iter()
                    .filter(|(entry, _)| &*entry.id == id)
                    .max_by(|(a, _), (b, _)| a.version.cmp(&b.version))
                    .and_then(|(_, slot)| slot.instance.downcast_ref::<Arc<E>>())
            })
            .cloned()
            .ok_or_else(|| RegistryError::NotFound {
                kind: kind_of::<E>(&maps),
                entry: EntityId::new(id),
            })
    }
}

/// The kind of the entities registered under `E`, or `Hook` as the least surprising
/// default before anything of that type was registered.
fn kind_of<E: ?Sized + 'static>(maps: &BTreeMap<TypeId, BTreeMap<EntityId, Slot>>) -> EntityKind {
    maps.get(&TypeId::of::<E>())
        .and_then(|map| map.values().next())
        .map_or(EntityKind::Hook, |slot| slot.kind)
}

#[cfg(test)]
mod tests {
    use super::{Entity, EntityId, EntityKind, Provenance, Registry, RegistryError};
    use std::sync::Arc;

    trait Tool: Entity {
        fn name(&self) -> &'static str;
    }

    struct Echo(EntityId);

    impl Entity for Echo {
        fn kind(&self) -> EntityKind {
            EntityKind::Tool
        }
        fn entity_id(&self) -> &EntityId {
            &self.0
        }
    }

    impl Tool for Echo {
        fn name(&self) -> &'static str {
            "echo"
        }
    }

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).expect("semver")
    }

    #[test]
    fn registers_and_resolves_the_same_instance() {
        let registry = Registry::new();
        let id = EntityId::versioned("echo", v("1.0.0"));
        let tool: Arc<dyn Tool> = Arc::new(Echo(id.clone()));
        registry
            .register(Arc::clone(&tool))
            .expect("first registration");
        let found = registry.get::<dyn Tool>(&id).expect("resolves");
        assert!(Arc::ptr_eq(&found, &tool));
        assert_eq!(found.name(), "echo");
    }

    #[test]
    fn a_second_registration_of_the_same_identity_is_refused_and_names_the_holder() {
        let registry = Registry::new();
        let id = EntityId::versioned("echo", v("1.0.0"));
        let first: Arc<dyn Tool> = Arc::new(Echo(id.clone()));
        let second: Arc<dyn Tool> = Arc::new(Echo(id.clone()));
        registry
            .register_with(first, Provenance::Plugin(EntityId::new("plugin-a")))
            .expect("first");
        let err = registry.register(second).expect_err("conflict");
        assert_eq!(
            err,
            RegistryError::Conflict {
                kind: EntityKind::Tool,
                entry: id,
                holder: Provenance::Plugin(EntityId::new("plugin-a")),
            }
        );
        assert!(err.to_string().contains("echo@1.0.0"));
    }

    #[test]
    fn replace_swaps_the_instance_and_returns_the_old_one() {
        let registry = Registry::new();
        let id = EntityId::new("echo");
        let first: Arc<dyn Tool> = Arc::new(Echo(id.clone()));
        let second: Arc<dyn Tool> = Arc::new(Echo(id.clone()));
        registry.register(Arc::clone(&first)).expect("first");
        let old = registry
            .replace(Arc::clone(&second))
            .expect("there was one");
        assert!(Arc::ptr_eq(&old, &first));
        assert!(Arc::ptr_eq(
            &registry.get::<dyn Tool>(&id).expect("new"),
            &second
        ));
        assert!(
            registry
                .replace::<dyn Tool>(Arc::new(Echo(EntityId::new("other"))))
                .is_none()
        );
    }

    #[test]
    fn a_missing_entity_is_a_typed_error_naming_kind_and_entry() {
        let registry = Registry::new();
        registry
            .register::<dyn Tool>(Arc::new(Echo(EntityId::new("echo"))))
            .expect("one");
        let Err(err) = registry.get::<dyn Tool>(&EntityId::new("nope")) else {
            panic!("a missing entity must not resolve")
        };
        assert_eq!(
            err,
            RegistryError::NotFound {
                kind: EntityKind::Tool,
                entry: EntityId::new("nope")
            }
        );
        assert_eq!(err.to_string(), "no Tool registered as nope");
    }

    #[test]
    fn latest_picks_the_highest_version_and_list_is_ordered() {
        let registry = Registry::new();
        for ver in ["1.2.0", "1.10.0", "0.9.0"] {
            registry
                .register::<dyn Tool>(Arc::new(Echo(EntityId::versioned("echo", v(ver)))))
                .expect("ok");
        }
        let latest = registry.latest::<dyn Tool>("echo").expect("some");
        assert_eq!(latest.entity_id().version, Some(v("1.10.0")));
        let ids: Vec<String> = registry
            .list::<dyn Tool>()
            .iter()
            .map(|t| t.entity_id().to_string())
            .collect();
        assert_eq!(ids, ["echo@0.9.0", "echo@1.2.0", "echo@1.10.0"]);
        assert!(registry.latest::<dyn Tool>("missing").is_err());
    }

    #[test]
    fn types_are_separate_maps() {
        trait Prompt: Entity {}
        struct P(EntityId);
        impl Entity for P {
            fn kind(&self) -> EntityKind {
                EntityKind::Prompt
            }
            fn entity_id(&self) -> &EntityId {
                &self.0
            }
        }
        impl Prompt for P {}
        let registry = Registry::new();
        registry
            .register::<dyn Tool>(Arc::new(Echo(EntityId::new("same"))))
            .expect("tool");
        registry
            .register::<dyn Prompt>(Arc::new(P(EntityId::new("same"))))
            .expect("prompt with the same id");
        assert_eq!(registry.list::<dyn Tool>().len(), 1);
        assert_eq!(registry.list::<dyn Prompt>().len(), 1);
    }
}
