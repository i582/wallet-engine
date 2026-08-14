import type {
  ProtectedSecretRead,
  ProtectedSecretRef,
  ProtectedSecretStore,
  ProtectedSecretStoreHost,
} from "../src"

export class MemorySecrets implements ProtectedSecretStoreHost {
  private readonly values: Map<string, Uint8Array> = new Map()

  async read(request: ProtectedSecretRead): Promise<Uint8Array> {
    const value = this.values.get(request.secretRef.value)
    if (!value) {
      throw hostError("notFound", "Secret not found")
    }
    return value.slice()
  }

  async store(request: ProtectedSecretStore): Promise<void> {
    this.values.set(request.secretRef.value, new Uint8Array(request.bytes))
  }

  async delete(secretRef: ProtectedSecretRef): Promise<void> {
    this.values.delete(secretRef.value)
  }
}

function hostError(kind: string, diagnostic: string): Error {
  return Object.assign(new Error(diagnostic), {kind, diagnostic})
}
