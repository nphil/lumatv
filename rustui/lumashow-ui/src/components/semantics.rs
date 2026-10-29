//! What a screen means, apart from how it looks: the names, values and actions of its controls and their focus order, recorded
//! even before an external accessibility bridge exists. A component describes itself into a [`SemanticsSink`]; hidden content
//! describes nothing, so it is as absent from navigation as it is from hit testing.

use crate::geometry::Frame;

/// What kind of thing a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticKind {
    Label,
    Button,
    Toggle,
    Slider,
    Progress,
    Choice,
    Row,
    List,
    Card,
    Key,
    TextField,
    Dialog,
    Sheet,
    Toast,
    Status,
    Page,
}

/// One described thing.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticNode {
    pub kind: SemanticKind,
    /// What it is called: the label a person would read out.
    pub name: String,
    /// Its value, when it has one: `On`, `42%`, the text of a field (never a secret's characters).
    pub value: Option<String>,
    pub frame: Frame,
    pub enabled: bool,
    pub selected: bool,
    /// Whether a touch on it does something.
    pub actionable: bool,
}

impl SemanticNode {
    pub fn new(kind: SemanticKind, name: impl Into<String>, frame: Frame) -> Self {
        Self {
            kind,
            name: name.into(),
            value: None,
            frame,
            enabled: true,
            selected: false,
            actionable: false,
        }
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn actionable(mut self) -> Self {
        self.actionable = true;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

/// Collects nodes in focus order: the order components describe themselves in.
#[derive(Debug, Default)]
pub struct SemanticsSink {
    nodes: Vec<SemanticNode>,
}

impl SemanticsSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, node: SemanticNode) {
        self.nodes.push(node);
    }

    pub fn nodes(&self) -> &[SemanticNode] {
        &self.nodes
    }

    pub fn into_nodes(self) -> Vec<SemanticNode> {
        self.nodes
    }

    /// The first node called `name`.
    pub fn find(&self, name: &str) -> Option<&SemanticNode> {
        self.nodes.iter().find(|n| n.name == name)
    }
}
