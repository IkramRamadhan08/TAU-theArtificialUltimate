use anyhow::Result;
use std::collections::HashMap;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update};

#[allow(dead_code)]
pub struct CrdtDocument {
    doc: Doc,
    buffer_id: u64,
}

impl CrdtDocument {
    pub fn new(buffer_id: u64) -> Self {
        let doc = Doc::new();
        Self { doc, buffer_id }
    }

    pub fn set_text(&self, text: &str) {
        let text_type = self.doc.get_or_insert_text("content");
        let mut txn = self.doc.transact_mut();
        text_type.push(&mut txn, text);
    }

    pub fn get_text(&self) -> String {
        let text_type = self.doc.get_or_insert_text("content");
        let txn = self.doc.transact();
        text_type.get_string(&txn)
    }

    pub fn apply_update(&self, data: &[u8]) -> Result<()> {
        let update = Update::decode_v1(data)?;
        let mut txn = self.doc.transact_mut();
        txn.apply_update(update);
        Ok(())
    }

    pub fn encode_state_vector(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        txn.state_vector().encode_v1()
    }

    pub fn encode_diff(&self, sv: &[u8]) -> Result<Vec<u8>> {
        let txn = self.doc.transact();
        let state_vector = yrs::StateVector::decode_v1(sv)
            .map_err(|e| anyhow::anyhow!("failed to decode state vector: {}", e))?;
        let update = txn.encode_diff_v1(&state_vector);
        Ok(update)
    }

    pub fn encode_all(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        txn.encode_state_as_update_v1(&yrs::StateVector::default())
    }
}

pub struct CrdtSync {
    documents: HashMap<u64, CrdtDocument>,
}

impl CrdtSync {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    pub fn get_or_create(&mut self, buffer_id: u64) -> &mut CrdtDocument {
        self.documents
            .entry(buffer_id)
            .or_insert_with(|| CrdtDocument::new(buffer_id))
    }

    pub fn get(&self, buffer_id: u64) -> Option<&CrdtDocument> {
        self.documents.get(&buffer_id)
    }

    pub fn sync_step1(&self, buffer_id: u64) -> Option<Vec<u8>> {
        self.documents
            .get(&buffer_id)
            .map(|doc| doc.encode_state_vector())
    }

    pub fn sync_step2(&self, buffer_id: u64, data: &[u8]) -> Option<Result<()>> {
        self.documents.get(&buffer_id).map(|doc| doc.apply_update(data))
    }

    pub fn remove(&mut self, buffer_id: u64) {
        self.documents.remove(&buffer_id);
    }
}
