# C ABI example

The example links the Wallet Engine C library and verifies that the generated
header and the native library report the same ABI version.

Build and run it from the repository root:

```shell
just example-c-run
```

Use `just example-c-build` when you only need to build it.

The header is generated from the separate `c-bindings` crate:

```shell
cargo xtask bindings c
```
