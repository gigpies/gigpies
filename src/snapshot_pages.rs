//! Coherent transport segmentation of one immutable serialized reply. Segments
//! never combine independent observations. 64 KiB frames and 1 MiB assembly budget.
use crate::show::Result;
use serde::{Deserialize, Serialize};
pub const ASSEMBLY_BYTES: usize = 1024 * 1024;
pub const SEGMENT_BYTES: usize = 8192;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub contract: String,
    pub version: u32,
    pub identity: String,
    pub index: usize,
    pub count: usize,
    pub total_bytes: usize,
    pub payload: String,
}
pub fn encode(bytes: Vec<u8>) -> Result<Vec<Vec<u8>>> {
    if bytes.len() <= 65536 {
        return Ok(vec![bytes]);
    }
    if bytes.len() > ASSEMBLY_BYTES {
        return Err("snapshot assembly admission: one MiB".into());
    }
    use sha2::{Digest, Sha256};
    let identity = crate::hash::hex(Sha256::digest(&bytes));
    // JSON producer emits ASCII for protocol strings and parameter names; UTF8
    // splits must nevertheless preserve codepoint boundaries for labels.
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + SEGMENT_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(&text[start..end]);
        start = end;
    }
    let count = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            serde_json::to_vec(&Page {
                contract: "GP14-snapshot-pages".into(),
                version: 1,
                identity: identity.clone(),
                index,
                count,
                total_bytes: text.len(),
                payload: payload.into(),
            })
            .map_err(|e| e.to_string())
        })
        .collect()
}
#[derive(Default)]
pub struct Assembly {
    pages: Vec<String>,
    identity: String,
    total: usize,
    count: usize,
    used: usize,
    started: Option<u64>,
}
impl Assembly {
    pub fn offer(&mut self, page: Page, now_ms: u64) -> Result<Option<Vec<u8>>> {
        let result = self.offer_inner(page, now_ms);
        if result.is_err() {
            *self = Self::default();
        }
        result
    }
    fn offer_inner(&mut self, page: Page, now_ms: u64) -> Result<Option<Vec<u8>>> {
        if page.contract != "GP14-snapshot-pages"
            || page.version != 1
            || page.total_bytes > ASSEMBLY_BYTES
            || page.total_bytes <= 65536
            || page.count == 0
            || page.count > ASSEMBLY_BYTES / SEGMENT_BYTES + 1
            || page.payload.len() > SEGMENT_BYTES
            || page.identity.len() != 64
            || page.index >= page.count
        {
            return Err("page admission".into());
        }
        if self
            .started
            .is_some_and(|t| now_ms < t || now_ms - t > 2000)
        {
            return Err("assembly expired".into());
        }
        if self.started.is_none() {
            if page.index != 0 {
                return Err("first page".into());
            }
            self.started = Some(now_ms);
            self.identity = page.identity.clone();
            self.total = page.total_bytes;
            self.count = page.count;
            self.pages = Vec::with_capacity(page.count);
        }
        if page.identity != self.identity
            || page.total_bytes != self.total
            || page.index != self.pages.len()
            || page.count != self.count
        {
            return Err("mixed/reordered snapshot pages".into());
        }
        self.used = self
            .used
            .checked_add(page.payload.len())
            .ok_or("assembly overflow")?;
        if self.used > self.total {
            return Err("assembly size".into());
        }
        self.pages.push(page.payload);
        if self.pages.len() != page.count {
            return Ok(None);
        }
        let bytes = self.pages.concat().into_bytes();
        use sha2::{Digest, Sha256};
        if bytes.len() != self.total || crate::hash::hex(Sha256::digest(&bytes)) != self.identity {
            return Err("snapshot checksum".into());
        }
        *self = Self::default();
        Ok(Some(bytes))
    }
}
