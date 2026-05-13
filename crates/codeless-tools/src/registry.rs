use std::collections::HashMap;
use std::sync::Arc;

use crate::tool::Tool;

/// Map of tool name → tool impl, owned by the host binary.
///
/// Built once at startup. Mutated via `register` until the host
/// hands a frozen `Arc<ToolRegistry>` to `codeless-mcp`. Tool-led
/// registration: the host calls each tool family's builder, which
/// pushes registrations in; this matches how plugins will later
/// register without `codeless-mcp` knowing about discovery.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. Returns the previous registration under the
    /// same name if any — callers that see `Some` are double-
    /// registering and should treat it as a bug.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Option<Arc<dyn Tool>> {
        let name = tool.name().to_string();
        self.tools.insert(name, tool)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
