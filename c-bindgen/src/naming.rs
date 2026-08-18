use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};

pub(super) fn type_name(rust_name: &str) -> String {
    format!("WalletEngine{}", rust_name.to_upper_camel_case())
}

pub(super) fn constant_name(rust_type: &str, rust_variant: &str) -> String {
    format!(
        "WALLET_ENGINE_{}_{}",
        rust_type.to_shouty_snake_case(),
        rust_variant.to_shouty_snake_case(),
    )
}

pub(super) fn function_name(rust_name: &str) -> String {
    rust_name.to_snake_case()
}

pub(super) fn field_name(rust_name: &str) -> String {
    rust_name.to_snake_case()
}

#[cfg(test)]
mod tests {
    use super::{constant_name, field_name, function_name, type_name};

    #[test]
    fn names_are_prefixed_and_stable() {
        assert_eq!(type_name("HttpMethod"), "WalletEngineHttpMethod");
        assert_eq!(
            constant_name("HttpMethod", "Post"),
            "WALLET_ENGINE_HTTP_METHOD_POST"
        );
        assert_eq!(function_name("HttpMethod"), "http_method");
        assert_eq!(field_name("requestTimeoutMs"), "request_timeout_ms");
    }
}
