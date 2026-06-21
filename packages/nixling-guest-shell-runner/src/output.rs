use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagementResult {
    Ok,
    AlreadyDetached,
    NotFound,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellManagementOutput {
    pub command: &'static str,
    pub name: String,
    pub result: ManagementResult,
}

impl ShellManagementOutput {
    pub fn unsupported(command: &'static str, name: String) -> Self {
        Self {
            command,
            name,
            result: ManagementResult::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ManagementResult, ShellManagementOutput};

    #[test]
    fn management_result_uses_stable_kebab_case() {
        let output = ShellManagementOutput {
            command: "detach",
            name: "default".to_owned(),
            result: ManagementResult::AlreadyDetached,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert_eq!(
            json,
            r#"{"command":"detach","name":"default","result":"already-detached"}"#
        );
    }
}
