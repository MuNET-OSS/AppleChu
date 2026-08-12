mod manifest;
mod parser;
mod value;

use std::path::Path;

use crate::{Schema, SchemaError};

pub fn generate_from_rust_dir(root: impl AsRef<Path>) -> Result<Schema, SchemaError> {
    let mut sections = parser::parse_directory(root.as_ref())?;
    sections.sort_by_key(|section| section.order.base10_parse::<u16>().unwrap_or(u16::MAX));
    Schema::parse(manifest::build(&sections)?)
}
