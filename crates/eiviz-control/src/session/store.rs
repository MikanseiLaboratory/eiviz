use crate::error::{ControlError, ControlResult};
use crate::session::Document;
use crate::session::validate::{validate, validate_for_apply};

#[derive(Debug, Clone)]
pub struct CanonicalSessionStore {
    document: Option<Document>,
    revision: u64,
}

impl Default for CanonicalSessionStore {
    fn default() -> Self {
        Self {
            document: None,
            revision: 0,
        }
    }
}

impl CanonicalSessionStore {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    pub fn document_cloned(&self) -> Option<Document> {
        self.document.clone()
    }

    pub fn replace(
        &mut self,
        document: Document,
        expected_revision: Option<u64>,
        for_apply: bool,
    ) -> ControlResult<(Document, u64, Option<Document>)> {
        if let Some(expected) = expected_revision {
            if self.document.is_some() && expected != self.revision {
                return Err(ControlError::conflict(format!(
                    "session revision {expected} does not match {}",
                    self.revision
                )));
            }
        }
        if for_apply {
            validate_for_apply(&document).map_err(|error| ControlError::invalid(error.message))?;
        } else {
            validate(&document).map_err(|error| ControlError::invalid(error.message))?;
        }
        let previous = self.document.clone();
        self.document = Some(document.clone());
        self.revision = self.revision.saturating_add(1);
        Ok((document, self.revision, previous))
    }

    pub fn rollback(&mut self, previous: Option<Document>, revision: u64) {
        self.document = previous;
        self.revision = revision;
    }
}
