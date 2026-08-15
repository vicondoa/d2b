pub fn format_number(value: usize) -> String {
    serde_json::to_string(&value).expect("serialize fixture number")
}
