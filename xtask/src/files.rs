use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

pub(crate) fn normalize_text(source: &str) -> String {
    let normalized = source
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{normalized}\n")
}

pub(crate) fn normalize_file(path: &Path) -> Result<()> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    fs::write(path, normalize_text(&source))
        .with_context(|| format!("failed to normalize {}", path.display()))
}

pub(crate) fn require_file(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(anyhow!(
            "UniFFI did not generate expected file: {}",
            path.display()
        ))
    }
}

pub(crate) fn copy_generated(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("output has no parent directory: {}", destination.display()))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create binding output directory {}",
            parent.display()
        )
    })?;
    fs::copy(source, destination)
        .map(|_| ())
        .with_context(|| format!("failed to copy to {}", destination.display()))
}

#[cfg(test)]
mod tests {
    use super::normalize_text;

    #[test]
    fn normalizes_horizontal_whitespace_and_final_newline() {
        assert_eq!(normalize_text("one  \n two\t\n"), "one\n two\n");
    }
}
