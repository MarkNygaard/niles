//! Tool trait + descriptor.

use crate::error::Result;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    /// JSON-Schema object describing tool parameters.
    pub parameters: Value,
}

impl ToolDescriptor {
    /// Convert to the wire-shape expected by `niles_llm::Tool`.
    pub fn to_llm_tool(&self) -> niles_llm::Tool {
        niles_llm::Tool {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;
    async fn execute(&self, args: Value) -> Result<Value>;

    /// Whether the model is offered it now. Asked at every request, so a
    /// tool that hangs on a setting — the web search on its add-on —
    /// comes and goes with it without the registry being built again.
    fn offered(&self) -> bool {
        true
    }
}
