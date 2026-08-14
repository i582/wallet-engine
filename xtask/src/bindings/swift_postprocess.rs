use anyhow::{Result, anyhow, bail};

pub(crate) fn postprocess_swift(source: &str) -> Result<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;
    let mut counts = RewriteCounts::default();

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if line == "import wallet_engineFFI" {
            output.push("@preconcurrency import wallet_engineFFI".to_owned());
            counts.imports += 1;
        } else if trimmed == "let makeCall = {" {
            rewrite_make_call(&lines, &mut output, &mut index, indent, &mut counts)?;
        } else if let Some(value_type) = trimmed
            .strip_prefix("let uniffiHandleSuccess = { (returnValue: ")
            .and_then(|value| value.strip_suffix(") in"))
        {
            output.push(format!(
                "{indent}let uniffiHandleSuccess: @Sendable ({value_type}) -> Void = {{ (returnValue: {value_type}) in"
            ));
            counts.successes += 1;
        } else if trimmed == "let uniffiHandleError = { (statusCode, errorBuf) in" {
            output.push(format!(
                "{indent}let uniffiHandleError: @Sendable (Int8, RustBuffer) -> Void = {{ (statusCode, errorBuf) in"
            ));
            counts.errors += 1;
        } else if line == "private func uniffiTraitInterfaceCallAsync<T>(" {
            output.push("private func uniffiTraitInterfaceCallAsync<T: Sendable>(".to_owned());
            counts.helpers += 1;
        } else if line == "private func uniffiTraitInterfaceCallAsyncWithError<T, E>(" {
            output.push(
                "private func uniffiTraitInterfaceCallAsyncWithError<T: Sendable, E: Sendable>("
                    .to_owned(),
            );
            counts.error_helpers += 1;
        } else {
            output.push(rewrite_helper_parameter(line));
        }
        index += 1;
    }

    counts.validate()?;
    Ok(format!("{}\n", output.join("\n")))
}

fn rewrite_make_call(
    lines: &[&str],
    output: &mut Vec<String>,
    index: &mut usize,
    indent: &str,
    counts: &mut RewriteCounts,
) -> Result<()> {
    let signature = lines
        .get(*index + 1)
        .ok_or_else(|| anyhow!("generated Swift ended after 'let makeCall = {{'"))?;
    let trimmed = signature.trim_start();
    if trimmed.starts_with("() async throws -> ") && trimmed.ends_with(" in") {
        let return_type = trimmed
            .strip_prefix("() async throws -> ")
            .and_then(|value| value.strip_suffix(" in"))
            .ok_or_else(|| anyhow!("invalid async callback signature"))?;
        output.push(format!(
            "{indent}let makeCall: @Sendable () async throws -> {return_type} = {{"
        ));
        counts.make_calls += 1;
    } else if trimmed.starts_with("() throws -> ") && trimmed.ends_with(" in") {
        output.push(format!("{indent}let makeCall = {{"));
    } else {
        bail!("UniFFI callback template changed after 'let makeCall = {{'");
    }
    output.push((*signature).to_owned());
    *index += 1;
    Ok(())
}

fn rewrite_helper_parameter(line: &str) -> String {
    line.replace(
        "makeCall: @escaping () async throws -> T,",
        "makeCall: @escaping @Sendable () async throws -> T,",
    )
    .replace(
        "handleSuccess: @escaping (T) -> (),",
        "handleSuccess: @escaping @Sendable (T) -> (),",
    )
    .replace(
        "handleError: @escaping (Int8, RustBuffer) -> (),",
        "handleError: @escaping @Sendable (Int8, RustBuffer) -> (),",
    )
    .replace(
        "lowerError: @escaping (E) -> RustBuffer,",
        "lowerError: @escaping @Sendable (E) -> RustBuffer,",
    )
}

#[derive(Default)]
struct RewriteCounts {
    imports: usize,
    make_calls: usize,
    successes: usize,
    errors: usize,
    helpers: usize,
    error_helpers: usize,
}

impl RewriteCounts {
    fn validate(&self) -> Result<()> {
        if self.imports != 1 {
            return Err(anyhow!(
                "expected one wallet_engineFFI import, found {}",
                self.imports
            ));
        }
        if self.make_calls == 0
            || self.make_calls != self.successes
            || self.make_calls != self.errors
        {
            return Err(anyhow!(
                "incomplete async callback rewrite: makeCall={}, success={}, error={}",
                self.make_calls,
                self.successes,
                self.errors
            ));
        }
        if self.helpers != 1 || self.error_helpers != 1 {
            bail!("expected both UniFFI async callback helpers exactly once");
        }
        Ok(())
    }
}
