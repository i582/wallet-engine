# UniFFI 0.32 generates callback closures without the Sendable annotations
# required by Swift 6 strict concurrency. Keep this mechanical transform next
# to the V3 generator so committed bindings are reproducible.

function fail(message) {
    print "error: " message > "/dev/stderr"
    exit 1
}

BEGIN {
    import_count = 0
    make_call_count = 0
    success_count = 0
    error_count = 0
    helper_count = 0
    helper_with_error_count = 0
    pending_make_call = 0
}

pending_make_call {
    signature = $0
    if (signature ~ /^[[:space:]]*\(\) throws -> .+ in$/) {
        # Synchronous callback methods use the same opening line. They do not
        # create a Task and need no Sendable rewrite.
        print make_call_indent "let makeCall = {"
        print signature
        pending_make_call = 0
        next
    }
    if (signature !~ /^[[:space:]]*\(\) async throws -> .+ in$/) {
        fail("UniFFI callback template changed after 'let makeCall = {'")
    }

    return_type = signature
    sub(/^[[:space:]]*\(\) async throws -> /, "", return_type)
    sub(/ in$/, "", return_type)

    print make_call_indent "let makeCall: @Sendable () async throws -> " return_type " = {"
    print signature
    pending_make_call = 0
    make_call_count++
    next
}

$0 == "import wallet_engine_v3FFI" {
    print "@preconcurrency import wallet_engine_v3FFI"
    import_count++
    next
}

/^[[:space:]]*let makeCall = \{$/ {
    make_call_indent = $0
    sub(/let makeCall = \{$/, "", make_call_indent)
    pending_make_call = 1
    next
}

/^[[:space:]]*let uniffiHandleSuccess = \{ \(returnValue: .+\) in$/ {
    line = $0
    indent = line
    sub(/let uniffiHandleSuccess =.*$/, "", indent)

    value_type = line
    sub(/^.*\(returnValue: /, "", value_type)
    sub(/\) in$/, "", value_type)

    print indent "let uniffiHandleSuccess: @Sendable (" value_type ") -> Void = { (returnValue: " value_type ") in"
    success_count++
    next
}

/^[[:space:]]*let uniffiHandleError = \{ \(statusCode, errorBuf\) in$/ {
    line = $0
    indent = line
    sub(/let uniffiHandleError =.*$/, "", indent)
    print indent "let uniffiHandleError: @Sendable (Int8, RustBuffer) -> Void = { (statusCode, errorBuf) in"
    error_count++
    next
}

$0 == "private func uniffiTraitInterfaceCallAsync<T>(" {
    print "private func uniffiTraitInterfaceCallAsync<T: Sendable>("
    helper_count++
    next
}

$0 == "private func uniffiTraitInterfaceCallAsyncWithError<T, E>(" {
    print "private func uniffiTraitInterfaceCallAsyncWithError<T: Sendable, E: Sendable>("
    helper_with_error_count++
    next
}

/^[[:space:]]*makeCall: @escaping \(\) async throws -> T,$/ {
    sub(/@escaping \(\)/, "@escaping @Sendable ()")
    print
    next
}

/^[[:space:]]*handleSuccess: @escaping \(T\) -> \(\),$/ {
    sub(/@escaping \(T\)/, "@escaping @Sendable (T)")
    print
    next
}

/^[[:space:]]*handleError: @escaping \(Int8, RustBuffer\) -> \(\),$/ {
    sub(/@escaping \(Int8, RustBuffer\)/, "@escaping @Sendable (Int8, RustBuffer)")
    print
    next
}

/^[[:space:]]*lowerError: @escaping \(E\) -> RustBuffer,$/ {
    sub(/@escaping \(E\)/, "@escaping @Sendable (E)")
    print
    next
}

{ print }

END {
    if (pending_make_call) {
        fail("generated Swift ended after 'let makeCall = {'")
    }
    if (import_count != 1) {
        fail("expected one wallet_engine_v3FFI import, found " import_count)
    }
    if (make_call_count == 0 || make_call_count != success_count || make_call_count != error_count) {
        fail("incomplete async callback rewrite: makeCall=" make_call_count ", success=" success_count ", error=" error_count)
    }
    if (helper_count != 1 || helper_with_error_count != 1) {
        fail("expected both UniFFI async callback helpers exactly once")
    }
}
