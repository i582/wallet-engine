# TypeScript provider transport examples

The example puts each provider transport in its own file:

- [`http-transport.ts`](http-transport.ts) creates a client backed by the
  built-in HTTP host.
- [`relay-transport.ts`](relay-transport.ts) adapts an application relay that
  returns only body or error. Its runnable `BodyOnlyFetchRelay` makes real
  requests while deliberately discarding HTTP metadata.
- [`demo.ts`](demo.ts) creates only the transport selected on the command line.

All wallet operations after construction are identical; only the provider
boundary changes.

Choose the HTTP client when the host can return complete HTTP metadata:

```ts
const client = await createHttpClient(config, platformHost, {toncenterApiKey})
```

Choose the relay client when the host returns only body or error. Adapt the
logical request to the application's protocol:

```ts
const relay: ProviderRelay = {
  async execute(request, signal) {
    const result = await applicationProxy.invoke(
      {
        requestId: request.id.value,
        url: request.url,
        method: request.method,
        headers: request.headers,
        body: request.body,
      },
      {signal},
    )

    // This is the original provider body, not an HTTP response envelope.
    return Uint8Array.from(result.body)
  },

  async cancel(requestId) {
    await applicationProxy.cancel(requestId)
  },
}

const client = await createRelayClient(config, platformHost, relay)
```

`RelayProviderHost` enforces `request.timeoutMs`, bounds response bodies,
supports cancellation before and during execution, and maps opaque failures to
`{kind, diagnostic}`. It deliberately does not fabricate a status, headers, or
final URL. The relay must not follow or emulate provider redirects.

Generate the WASM bindings, install this example package, and type-check it
with Bun:

```shell
just bindings-wasm
bun install --cwd examples/provider-transports-ts --frozen-lockfile
bun --cwd examples/provider-transports-ts check
```

Run [`demo.ts`](demo.ts) with the transport you want. Both commands create a
temporary testnet wallet and make real Toncenter v2 requests. The generated
wallet is kept only in memory:

```shell
bun --cwd examples/provider-transports-ts http
bun --cwd examples/provider-transports-ts relay
```

`BodyOnlyFetchRelay` exists to keep the relay example directly runnable.
Replace it with the application's actual `ProviderRelay` implementation; the
`createRelayClient` call remains unchanged.
