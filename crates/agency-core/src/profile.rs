use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl AgentProfile {
    /// Render `args`, replacing the literal token `{{prompt}}` with `prompt`.
    pub fn render_args(&self, prompt: &str) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.replace("{{prompt}}", prompt))
            .collect()
    }
}
