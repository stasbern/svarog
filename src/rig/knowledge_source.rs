use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Namespace {
    Factual,
    Project,
    Personal,
    Tmp,
}

impl Namespace {
    pub fn table_name(&self) -> &'static str {
        match self {
            Namespace::Factual  => "svarog_factual",
            Namespace::Project  => "svarog_project",
            Namespace::Personal => "svarog_personal",
            Namespace::Tmp      => "svarog_tmp",
        }
    }

    /// All namespaces that should be searched (excludes Tmp)
    pub fn searchable() -> &'static [Namespace] {
        &[Namespace::Factual, Namespace::Project, Namespace::Personal]
    }

    
    pub fn parse(text: &str) -> Namespace {
        let lower = text.to_lowercase();
        if lower.contains("project")  { Namespace::Project }
        else if lower.contains("personal") { Namespace::Personal }
        else { Namespace::Factual } // default: factual
    }

    /// Parse LLM classifier output like "factual, project" into namespaces
    pub fn parse_list(text: &str) -> Vec<Namespace> {
        let mut result = Vec::new();
        let lower = text.to_lowercase();
        if lower.contains("factual")  { result.push(Namespace::Factual); }
        if lower.contains("project")  { result.push(Namespace::Project); }
        if lower.contains("personal") { result.push(Namespace::Personal); }
        if result.is_empty() {
            // Fallback: search factual + project
            vec![Namespace::Factual, Namespace::Project]
        } else {
            result
        }
    }

        /// System prompt for the ingestion classifier
    pub fn classifier_prompt() -> &'static str {
        "You classify documents into exactly one knowledge category. \
         Respond with ONLY one word from: factual, project, personal.\n\n\
         - factual: documentation, reference material, manuals, technical specs, how-to guides\n\
         - project: code, architecture decisions, project plans, work tasks\n\
         - personal: preferences, goals, journal entries, personal notes\n\n\
         No explanation. One word only."
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Namespace::Factual  => write!(f, "factual"),
            Namespace::Project  => write!(f, "project"),
            Namespace::Personal => write!(f, "personal"),
            Namespace::Tmp      => write!(f, "tmp"),
        }
    }
}

impl Default for Namespace {
    fn default() -> Self { Namespace::Factual }
}