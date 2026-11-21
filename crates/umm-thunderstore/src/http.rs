//! Network access. UMM identifies itself only as `UMM/<version>` and sends nothing else
//! about the user.

use crate::{Result, TsError};
use std::io::Read;
use std::time::Duration;

pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>>;
}

pub struct UreqHttp {
    agent: ureq::Agent,
}

/// Largest response UMM reads into memory (package zips can be large).
const MAX_BODY: u64 = 4 << 30;

impl Default for UreqHttp {
    fn default() -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(concat!("UMM/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(60))
            .build();
        UreqHttp { agent }
    }
}

fn net(url: &str, e: impl std::fmt::Display) -> TsError {
    TsError::Network {
        url: url.to_string(),
        message: e.to_string(),
    }
}

fn read_body(url: &str, resp: ureq::Response) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_BODY)
        .read_to_end(&mut buf)
        .map_err(|e| net(url, e))?;
    Ok(buf)
}

impl Http for UreqHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.agent.get(url).call().map_err(|e| net(url, e))?;
        read_body(url, resp)
    }

    fn post(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        let resp = self
            .agent
            .post(url)
            .set("Content-Type", content_type)
            .send_bytes(body)
            .map_err(|e| net(url, e))?;
        read_body(url, resp)
    }
}

/// Answers from a fixed map; for tests.
#[derive(Default)]
pub struct FakeHttp {
    pub responses: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    pub requests: std::sync::Mutex<Vec<String>>,
}

impl FakeHttp {
    pub fn with(self, url: &str, body: impl Into<Vec<u8>>) -> Self {
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_string(), body.into());
        self
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.requests.lock().unwrap().push(url.to_string());
        self.responses
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| net(url, "no such page (test)"))
    }

    fn post(&self, url: &str, _content_type: &str, _body: &[u8]) -> Result<Vec<u8>> {
        self.get(url)
    }
}
