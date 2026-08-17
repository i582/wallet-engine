pub(super) fn render(source: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = source.to_owned();
    for (name, value) in replacements {
        let placeholder = format!("{{{{{name}}}}}");
        assert!(
            rendered.contains(&placeholder),
            "template does not contain placeholder {placeholder}"
        );
        rendered = rendered.replace(&placeholder, value);
    }
    assert!(
        !rendered.contains("{{"),
        "template contains an unresolved placeholder"
    );
    rendered
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn replaces_named_placeholders() {
        assert_eq!(
            render(
                "typedef {{TYPE}} {{NAME}};\n",
                &[("TYPE", "uint32_t"), ("NAME", "Value")]
            ),
            "typedef uint32_t Value;\n"
        );
    }

    #[test]
    #[should_panic(expected = "unresolved placeholder")]
    fn rejects_unresolved_placeholders() {
        render("{{VALUE}}", &[]);
    }
}
