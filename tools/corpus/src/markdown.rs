// Text read from the results ends up in the run's Markdown summary. Every
// decision is made on the value as recorded; only what is written there goes
// through clean().

// Keeps ASCII letters and digits, space and . , - _ : / + ( ), which cannot
// start Markdown or HTML; anything else, line breaks and tabs included,
// becomes '?'.
pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() || " .,-_:/+()".contains(c) { c } else { '?' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_characters_survive() {
        assert_eq!(clean("a\nb|c<d>`e` é.,-_:/+()\t#[x](y)@z!"), "a?b?c?d??e? ?.,-_:/+()???x?(y)?z?");
        assert_eq!(clean("universal:x86_64,aarch64"), "universal:x86_64,aarch64");
        assert_eq!(clean("OOX/binary file converter. Version: 9.4.0.129"), "OOX/binary file converter. Version: 9.4.0.129");
    }
}
