//! Prompts and strings as assets: prompt fragments from markdown files, the string
//! catalog from `strings/<locale>.json` (R33).
//!
//! Text bound for a model is a [`PromptFragment`] (a file under `prompts/`, registered
//! as an entity) rendered into a [`Prompt`]; text bound for a person is a [`StringKey`]
//! looked up in the [`Catalog`]. Neither can be a string literal in code: `Prompt` has
//! no constructor from one, and `StringKey`s are generated from `strings/en.json`.

mod catalog;
mod prompt;

pub use catalog::{
    CATALOG_SOURCES, Catalog, FmtArgs, FmtValue, PatternError, StringKey, format, keys,
    placeholders,
};
pub use prompt::{
    DataRole, PROMPT_SOURCES, Prompt, PromptArgs, PromptError, PromptFileError, PromptFragment,
    PromptRegistry, PromptSegment,
};
