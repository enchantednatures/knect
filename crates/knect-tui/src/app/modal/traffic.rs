//! Traffic-split editor modal state and topology seeding.

use knect_proto::ResourceId;
use knect_proto::graph::Topology;
use knect_proto::rpc::{SetTrafficSplitParams, TrafficSplit};

use super::seed::{seed_traffic, validate_tag};

/// Traffic editor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficRow {
    /// Revision name.
    pub revision: String,
    /// Percentage of traffic.
    pub percent: u8,
    /// Optional traffic tag.
    pub tag: String,
}

/// In-progress text edit in the traffic modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrafficEdit {
    /// Editing the selected row's percentage.
    Percent(String),
    /// Editing the selected row's tag.
    Tag(String),
}

/// Traffic-split editor modal.
#[derive(Debug, Clone, PartialEq)]
pub struct TrafficModal {
    /// Knative Service being edited.
    pub service: ResourceId,
    /// Owning Route, when discovered.
    pub route: Option<ResourceId>,
    /// Editable traffic rows.
    pub rows: Vec<TrafficRow>,
    /// Selected row.
    pub selected: usize,
    /// In-progress edit.
    pub edit: Option<TrafficEdit>,
    /// Seed or validation error.
    pub error: Option<String>,
    /// Last action result message.
    pub result: Option<String>,
    /// True while a dry run or submit is in flight.
    pub loading: bool,
    /// True when a second `s` confirms the write.
    pub confirm_pending: bool,
}

impl TrafficModal {
    /// Builds a modal seeded from a topology projection.
    #[must_use]
    pub fn from_topology(topology: &Topology, selected: &ResourceId) -> Self {
        let mut modal = Self {
            service: selected.clone(),
            route: None,
            rows: Vec::new(),
            selected: 0,
            edit: None,
            error: None,
            result: None,
            loading: false,
            confirm_pending: false,
        };
        match seed_traffic(topology, selected) {
            Ok(seed) => {
                modal.service = seed.service;
                modal.route = seed.route;
                modal.rows = seed.rows;
                if modal.rows.is_empty() {
                    modal.error = Some("the Route has no traffic entries".to_owned());
                }
            }
            Err(error) => modal.error = Some(error),
        }
        modal
    }

    /// Reseeds rows from a fresh topology.
    pub fn refresh(&mut self, topology: &Topology) {
        match seed_traffic(topology, &self.service) {
            Ok(seed) => {
                self.service = seed.service;
                self.route = seed.route;
                self.rows = seed.rows;
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
                self.error = None;
            }
            Err(error) => {
                self.rows.clear();
                self.error = Some(error);
            }
        }
        self.result = None;
        self.edit = None;
        self.confirm_pending = false;
    }

    /// Sum of the current percentages, including an in-progress edit.
    #[must_use]
    pub fn sum(&self) -> u32 {
        self.rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let percent = match (&self.edit, index == self.selected) {
                    (Some(TrafficEdit::Percent(buffer)), true) => {
                        buffer.parse::<u8>().unwrap_or(row.percent)
                    }
                    _ => row.percent,
                };
                u32::from(percent)
            })
            .sum()
    }

    /// True when the rows are a legal Knative traffic split.
    pub fn validate(&self) -> Result<(), String> {
        if self.rows.is_empty() {
            return Err("traffic split must contain at least one revision".to_owned());
        }
        let mut revisions = std::collections::BTreeSet::new();
        let mut tags = std::collections::BTreeSet::new();
        for row in &self.rows {
            if !revisions.insert(row.revision.as_str()) {
                return Err(format!("revision {} appears more than once", row.revision));
            }
            if let Err(error) = validate_tag(&row.tag) {
                return Err(format!("revision {}: {error}", row.revision));
            }
            if !row.tag.is_empty() && !tags.insert(row.tag.as_str()) {
                return Err(format!("traffic tag '{}' appears more than once", row.tag));
            }
        }
        let total = self.sum();
        if total != 100 {
            return Err(format!("traffic percentages must sum to 100, got {total}"));
        }
        Ok(())
    }

    /// Current rows as RPC splits.
    #[must_use]
    pub fn splits(&self) -> Vec<TrafficSplit> {
        self.rows
            .iter()
            .map(|row| TrafficSplit {
                revision: row.revision.clone(),
                percent: row.percent,
                tag: (!row.tag.is_empty()).then(|| row.tag.clone()),
            })
            .collect()
    }

    /// Request parameters for a dry run or a write.
    #[must_use]
    pub fn params(&self, dry_run: bool) -> Option<SetTrafficSplitParams> {
        let namespace = self.service.namespace.clone()?;
        Some(SetTrafficSplitParams {
            namespace,
            service: self.service.name.clone(),
            splits: self.splits(),
            dry_run: Some(dry_run),
        })
    }

    /// Selects the next row, clamped.
    pub fn select_next(&mut self, delta: i32) {
        if self.rows.is_empty() {
            self.selected = 0;
            return;
        }
        let target = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.selected
                .saturating_add(delta as usize)
                .min(self.rows.len() - 1)
        };
        self.selected = target;
        self.edit = None;
        self.confirm_pending = false;
    }

    /// Adjusts the selected percentage, clamping to 0..=100.
    pub fn adjust(&mut self, delta: i16) {
        let Some(row) = self.rows.get_mut(self.selected) else {
            return;
        };
        row.percent = (i16::from(row.percent) + delta).clamp(0, 100) as u8;
        self.confirm_pending = false;
    }

    /// Starts or extends percentage editing.
    pub fn edit_percent(&mut self, digit: char) {
        match self.edit.take() {
            Some(TrafficEdit::Percent(mut buffer)) if buffer.len() < 3 => {
                buffer.push(digit);
                self.edit = Some(TrafficEdit::Percent(buffer));
            }
            _ => self.edit = Some(TrafficEdit::Percent(digit.to_string())),
        }
        self.confirm_pending = false;
    }

    /// Starts tag editing for the selected row.
    pub fn edit_tag(&mut self) {
        let current = self
            .rows
            .get(self.selected)
            .map(|row| row.tag.clone())
            .unwrap_or_default();
        self.edit = Some(TrafficEdit::Tag(current));
        self.confirm_pending = false;
    }

    /// Appends a character to the active edit.
    pub fn push_char(&mut self, character: char) {
        match self.edit.as_mut() {
            Some(TrafficEdit::Percent(buffer))
                if character.is_ascii_digit() && buffer.len() < 3 =>
            {
                buffer.push(character);
            }
            Some(TrafficEdit::Tag(buffer)) if buffer.len() < 63 && !character.is_control() => {
                buffer.push(character);
            }
            _ => {}
        }
    }

    /// Removes the last character from the active edit.
    pub fn backspace(&mut self) {
        match self.edit.as_mut() {
            Some(TrafficEdit::Percent(buffer)) | Some(TrafficEdit::Tag(buffer)) => {
                buffer.pop();
            }
            None => {}
        }
    }

    /// Commits the active edit.
    pub fn commit_edit(&mut self) -> Result<(), String> {
        match self.edit.take() {
            None => Ok(()),
            Some(TrafficEdit::Percent(buffer)) => {
                let value: u8 = buffer
                    .parse()
                    .map_err(|_| "percentage must be a number from 0 to 100".to_owned())?;
                if value > 100 {
                    return Err(format!("percentage {value} exceeds 100"));
                }
                if let Some(row) = self.rows.get_mut(self.selected) {
                    row.percent = value;
                }
                Ok(())
            }
            Some(TrafficEdit::Tag(buffer)) => {
                let tag = buffer.trim().to_owned();
                validate_tag(&tag)?;
                if let Some(row) = self.rows.get_mut(self.selected) {
                    row.tag = tag;
                }
                Ok(())
            }
        }
    }

    /// Discards the active edit.
    pub fn cancel_edit(&mut self) {
        self.edit = None;
    }
}
