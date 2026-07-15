use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::ingestion::ExtractedDocument;
use super::knowledge_source::Namespace;

pub const DESCRIPTOR_VERSION: &str = "document-descriptor-v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribedEntity {
    #[serde(default)]
    pub name: String,

    #[serde(default)]
    pub entity_type: Option<String>,

    #[serde(default)]
    pub relation: Option<String>,

    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentDescriptor {
    #[serde(default)]
    pub title: String,

    #[serde(default)]
    pub summary: String,

    #[serde(default)]
    pub document_type: Option<String>,

    #[serde(default)]
    pub aliases: Vec<String>,

    #[serde(default)]
    pub identifiers: Vec<String>,

    #[serde(default)]
    pub topics: Vec<String>,

    #[serde(default)]
    pub entities: Vec<DescribedEntity>,

    #[serde(default)]
    pub attributes: Map<String, Value>,
}

impl DocumentDescriptor {
    pub fn extractor_prompt() -> &'static str {
        r#"
You extract a compact, domain-independent catalog descriptor for a document.

Return ONLY valid JSON with exactly this general structure:

{
  "title": "specific human-readable title",
  "summary": "one or two sentence description",
  "document_type": "generic type or null",
  "aliases": ["alternative names for the primary subject or document"],
  "identifiers": ["document numbers, model numbers, ISBNs, part numbers, standards, case numbers"],
  "topics": ["important searchable topics"],
  "entities": [
    {
      "name": "entity name",
      "entity_type": "generic type or null",
      "relation": "relationship to the document or primary subject, or null",
      "aliases": ["alternative entity names"]
    }
  ],
  "attributes": {
    "any_useful_generic_key": "any JSON value"
  }
}

Rules:
- Be domain-independent.
- Do not invent information.
- Prefer exact names and identifiers found in the input.
- The title should identify the document's actual subject, not merely say "manual".
- Put unusual domain-specific metadata inside attributes.
- Keep topics concise.
- Keep at most 12 entities and 20 topics.
- If a field is unknown, use null, an empty string, an empty array, or an empty object.
- Do not include Markdown fences or explanations.
"#
    }

    pub fn fallback(source_title: &str) -> Self {
        Self {
            title: source_title.to_string(),
            summary: format!("Document imported from {source_title}."),
            ..Default::default()
        }
    }

    pub fn from_model_output(
        response: &str,
        source_title: &str,
    ) -> Result<Self, serde_json::Error> {
        // Be tolerant of models that still add Markdown fences or commentary.
        let json = match (response.find('{'), response.rfind('}')) {
            (Some(start), Some(end)) if end >= start => &response[start..=end],
            _ => response.trim(),
        };

        let mut descriptor: Self = serde_json::from_str(json)?;
        descriptor.normalize(source_title);

        Ok(descriptor)
    }

    pub fn normalize(&mut self, source_title: &str) {
        self.title = self.title.trim().to_string();
        self.summary = self.summary.trim().to_string();

        if self.title.is_empty() {
            self.title = source_title.to_string();
        }

        if self.summary.is_empty() {
            self.summary = format!("Document about {}.", self.title);
        }

        self.document_type = self
            .document_type
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        normalize_strings(&mut self.aliases);
        normalize_strings(&mut self.identifiers);
        normalize_strings(&mut self.topics);

        self.entities
            .retain(|entity| !entity.name.trim().is_empty());

        for entity in &mut self.entities {
            entity.name = entity.name.trim().to_string();

            entity.entity_type = entity
                .entity_type
                .take()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());

            entity.relation = entity
                .relation
                .take()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());

            normalize_strings(&mut entity.aliases);
        }

        self.entities.truncate(12);
        self.topics.truncate(20);
    }

    pub fn searchable_text(&self) -> String {
        let mut lines = vec![
            format!("Title: {}", self.title),
            format!("Summary: {}", self.summary),
        ];

        if let Some(document_type) = &self.document_type {
            lines.push(format!("Document type: {document_type}"));
        }

        if !self.aliases.is_empty() {
            lines.push(format!("Aliases: {}", self.aliases.join("; ")));
        }

        if !self.identifiers.is_empty() {
            lines.push(format!("Identifiers: {}", self.identifiers.join("; ")));
        }

        if !self.topics.is_empty() {
            lines.push(format!("Topics: {}", self.topics.join("; ")));
        }

        if !self.entities.is_empty() {
            let entities = self
                .entities
                .iter()
                .map(|entity| {
                    let mut text = entity.name.clone();

                    if let Some(entity_type) = &entity.entity_type {
                        text.push_str(&format!(" ({entity_type})"));
                    }

                    if let Some(relation) = &entity.relation {
                        text.push_str(&format!(" [{relation}]"));
                    }

                    if !entity.aliases.is_empty() {
                        text.push_str(&format!(" aliases: {}", entity.aliases.join(", ")));
                    }

                    text
                })
                .collect::<Vec<_>>()
                .join("; ");

            lines.push(format!("Entities: {entities}"));
        }

        if !self.attributes.is_empty() {
            if let Ok(attributes) = serde_json::to_string(&self.attributes) {
                lines.push(format!("Attributes: {attributes}"));
            }
        }

        lines.join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDocument {
    pub document_key: String,
    pub source_path: String,
    pub raw_hash: String,
    pub media_type: String,
    pub page_count: usize,
    pub namespace: Namespace,

    pub descriptor: DocumentDescriptor,
    pub descriptor_text: String,

    pub descriptor_model: String,
    pub descriptor_version: String,
    pub ingestion_version: String,
}

impl KnowledgeDocument {
    pub fn from_extracted(
        extracted: &ExtractedDocument,
        namespace: Namespace,
        mut descriptor: DocumentDescriptor,
        descriptor_model: impl Into<String>,
        ingestion_version: impl Into<String>,
    ) -> Self {
        descriptor.normalize(&extracted.title);

        let descriptor_text = descriptor.searchable_text();

        Self {
            document_key: document_key(&extracted.source_path),
            source_path: extracted.source_path.clone(),
            raw_hash: extracted.raw_hash.clone(),
            media_type: extracted.media_type.clone(),
            page_count: extracted.page_count(),
            namespace,
            descriptor,
            descriptor_text,
            descriptor_model: descriptor_model.into(),
            descriptor_version: DESCRIPTOR_VERSION.to_string(),
            ingestion_version: ingestion_version.into(),
        }
    }

    pub fn catalog_context(&self) -> String {
        format!(
            "{}\nNamespace: {}\nPages: {}\nSource: {}",
            self.descriptor_text, self.namespace, self.page_count, self.source_path
        )
    }
}

#[derive(Debug, Clone)]
pub struct DocumentSearchResult {
    pub score: f64,
    pub document: KnowledgeDocument,
}

fn document_key(source_path: &str) -> String {
    let digest = Sha256::digest(source_path.as_bytes());

    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_strings(values: &mut Vec<String>) {
    let mut normalized = Vec::new();

    for value in values.drain(..) {
        let value = value.trim().to_string();

        if value.is_empty() {
            continue;
        }

        let already_present = normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&value));

        if !already_present {
            normalized.push(value);
        }
    }

    *values = normalized;
}
