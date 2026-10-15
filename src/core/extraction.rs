#[path = "extraction/extract.rs"]
pub mod extract;
#[path = "extraction/patch.rs"]
pub mod patch;
#[cfg(test)]
#[path = "extraction/test_support.rs"]
pub(crate) mod test_support;
