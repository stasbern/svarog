// src/rig/ingestion.rs

use color_eyre::{
    Result,
    eyre::{WrapErr, eyre},
};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ExtractedPage {
    /// One-based physical PDF page number.
    pub number: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// Normalized absolute source path.
    pub source_path: String,
    pub title: String,
    pub media_type: String,
    pub raw_hash: String,
    pub pages: Vec<ExtractedPage>,
}

impl ExtractedDocument {
    pub fn canonical_text(&self) -> String {
        let mut output = String::new();

        for page in &self.pages {
            if page.text.trim().is_empty() {
                continue;
            }

            let _ = writeln!(output, "\n\n[[source-page:{}]]\n", page.number);
            output.push_str(page.text.trim());
            output.push('\n');
        }

        output
    }

    /// Text-only preview used for automatic knowledge classification.
    pub fn preview(&self, max_chars: usize) -> String {
        self.pages
            .iter()
            .flat_map(|page| page.text.chars().chain(std::iter::once('\n')))
            .take(max_chars)
            .collect()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

/// Extracts a supported file.
///
/// `Ok(None)` means the extension is not currently supported.
pub async fn extract_path(path: &Path) -> Result<Option<ExtractedDocument>> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !is_supported_extension(&extension) {
        return Ok(None);
    }

    let absolute_path = tokio::fs::canonicalize(path)
        .await
        .wrap_err_with(|| format!("failed to canonicalize {}", path.display()))?;

    let source_path = absolute_path.to_string_lossy().replace('\\', "/");

    let title = absolute_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled")
        .to_string();

    let bytes = tokio::fs::read(&absolute_path)
        .await
        .wrap_err_with(|| format!("failed to read {}", absolute_path.display()))?;

    let raw_hash = hash_bytes(&bytes);

    let (media_type, pages) = match extension.as_str() {
        "pdf" => {
            let display_path = absolute_path.display().to_string();

            // PDF extraction is CPU-bound, so do not block the Tokio executor.
            let extracted_pages = tokio::task::spawn_blocking(move || {
                pdf_extract::extract_text_from_mem_by_pages(&bytes)
                    .map_err(|error| error.to_string())
            })
            .await
            .wrap_err("PDF extraction worker panicked")?
            .map_err(|error| eyre!("failed to extract {display_path}: {error}"))?;

            let pages = extracted_pages
                .into_iter()
                .enumerate()
                .map(|(index, text)| ExtractedPage {
                    number: index + 1,
                    text: normalize_text(&text),
                })
                .collect::<Vec<_>>();

            if pages.iter().all(|page| page.text.trim().is_empty()) {
                return Err(eyre!(
                    "{} contains no extractable text; it probably requires OCR or vision extraction",
                    absolute_path.display()
                ));
            }

            ("application/pdf".to_string(), pages)
        }

        _ => {
            let text = String::from_utf8(bytes).wrap_err_with(|| {
                format!("{} is not valid UTF-8", absolute_path.display())
            })?;

            (
                media_type_for_extension(&extension).to_string(),
                vec![ExtractedPage {
                    number: 1,
                    text: normalize_text(&text),
                }],
            )
        }
    };

    Ok(Some(ExtractedDocument {
        source_path,
        title,
        media_type,
        raw_hash,
        pages,
    }))
}

fn is_supported_extension(extension: &str) -> bool {
    matches!(
        extension,
        "pdf"
            | "txt"
            | "md"
            | "markdown"
            | "rst"
            | "adoc"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "csv"
            | "rs"
    )
}

fn media_type_for_extension(extension: &str) -> &'static str {
    match extension {
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "csv" => "text/csv",
        "rs" => "text/x-rust",
        _ => "text/plain",
    }
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn hash_bytes(bytes: &[u8]) -> String {
    let result = Sha256::digest(bytes);
    result.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_text_preserves_page_markers() {
        let document = ExtractedDocument {
            source_path: "manual.pdf".into(),
            title: "Manual".into(),
            media_type: "application/pdf".into(),
            raw_hash: "hash".into(),
            pages: vec![
                ExtractedPage {
                    number: 1,
                    text: "First page".into(),
                },
                ExtractedPage {
                    number: 2,
                    text: "Second page".into(),
                },
            ],
        };

        let text = document.canonical_text();

        assert!(text.contains("[[source-page:1]]"));
        assert!(text.contains("[[source-page:2]]"));
        assert!(text.contains("First page"));
        assert!(text.contains("Second page"));
    }

    #[test]
    fn preview_does_not_include_internal_markers() {
        let document = ExtractedDocument {
            source_path: "manual.pdf".into(),
            title: "Manual".into(),
            media_type: "application/pdf".into(),
            raw_hash: "hash".into(),
            pages: vec![
                ExtractedPage {
                    number: 1,
                    text: "Page one".into(),
                },
                ExtractedPage {
                    number: 2,
                    text: "Page two".into(),
                },
            ],
        };

        let preview = document.preview(100);

        assert!(preview.contains("Page one"));
        assert!(preview.contains("Page two"));
        assert!(!preview.contains("source-page"));
    }
}