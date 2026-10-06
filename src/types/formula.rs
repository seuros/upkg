#[path = "formula/bottle.rs"]
pub mod bottle;
#[path = "formula/resolve.rs"]
pub mod resolve;
#[path = "formula/types.rs"]
pub mod types;

pub use bottle::{SelectedBottle, select_bottle};
pub use resolve::resolve_closure;
pub use types::{
    Bottle, BottleFile, BottleStable, Formula, FormulaUrls, KegOnly, RubySourceChecksum, SourceUrl,
    UsesFromMacos, Versions,
};

pub fn formula_token(name: &str) -> &str {
    if name.is_empty() {
        return "";
    }

    name.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("")
}

#[cfg(all(test, target_os = "macos"))]
#[path = "formula/tests.rs"]
mod tests;
