use crate::TransportCapabilities;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportButton {
    Callback { label: String, data: String },
    Url { label: String, url: String },
}

impl TransportButton {
    pub fn callback(label: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Callback {
            label: label.into(),
            data: data.into(),
        }
    }

    pub fn url(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::Url {
            label: label.into(),
            url: url.into(),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Callback { label, .. } | Self::Url { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportKeyboard {
    pub rows: Vec<Vec<TransportButton>>,
}

impl TransportKeyboard {
    pub fn new(rows: Vec<Vec<TransportButton>>) -> Self {
        Self { rows }
    }

    pub fn empty() -> Self {
        Self { rows: Vec::new() }
    }

    pub fn add_row(mut self, row: Vec<TransportButton>) -> Self {
        self.rows.push(row);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn button_count(&self) -> usize {
        self.rows.iter().map(Vec::len).sum()
    }

    pub fn fits(&self, capabilities: TransportCapabilities) -> bool {
        if !capabilities.supports_inline_keyboard && !self.is_empty() {
            return false;
        }
        if capabilities
            .max_keyboard_rows
            .is_some_and(|max| self.row_count() > max)
        {
            return false;
        }
        if capabilities
            .max_buttons_total
            .is_some_and(|max| self.button_count() > max)
        {
            return false;
        }
        if capabilities
            .max_buttons_per_row
            .is_some_and(|max| self.rows.iter().any(|row| row.len() > max))
        {
            return false;
        }
        true
    }
}
