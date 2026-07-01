use color_eyre::Result;

pub struct OllamaClient {
    client: Client,
    model: String,
    preamble: String,
    temperature: f64,
}

impl OllamaClient {
    pub fn new(model: &str, preamble: &str, temperature: f64) -> Self {
        Self {
            client: Client::from_env().unwrap(),
            model: String::from(model),
            preamble: String::from(preamble),
            temperature,
        }
    }
}
