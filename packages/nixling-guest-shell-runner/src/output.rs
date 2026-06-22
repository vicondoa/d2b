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
    pub command: String,
    pub name: String,
    pub result: ManagementResult,
}

impl ShellManagementOutput {
    pub fn ok(command: impl Into<String>, name: String) -> Self {
        Self {
            command: command.into(),
            name,
            result: ManagementResult::Ok,
        }
    }

    pub fn unsupported(command: impl Into<String>, name: String) -> Self {
        Self {
            command: command.into(),
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
            command: "detach".to_owned(),
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
