use color_eyre::Result;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Request {
    Prompt(String),
}

#[derive(Debug, Clone)]
pub enum Response {
    // TODO: streaming
    Token(String),
    CompleteResponse(String),
    ContextFound(Vec<(f64, String)>),
    // not used yet
    Error(String),
}

pub struct Channels {
    pub prompt_tx: mpsc::Sender<Request>,
    pub prompt_rx: mpsc::Receiver<Request>,
    pub response_tx: mpsc::Sender<Response>,
    pub response_rx: mpsc::Receiver<Response>,
}

impl Channels {
    pub fn new() -> Result<Self> {
        let (prompt_tx, prompt_rx) = mpsc::channel::<Request>(100);
        let (response_tx, response_rx) = mpsc::channel::<Response>(100);
        Ok(Self {
            prompt_tx,
            prompt_rx,
            response_tx,
            response_rx,
        })
    }
}
