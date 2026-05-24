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
}
