pub fn format_number(value: usize) -> String {
    itoa::Buffer::new().format(value).to_owned()
}

#[cfg(test)]
mod tests {
    use super::format_number;

    #[test]
    fn formats_a_number_through_the_external_crate() {
        assert_eq!(format_number(42), "42");
    }
}
