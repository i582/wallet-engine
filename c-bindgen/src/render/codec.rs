use crate::{model::BindingsModel, type_map::BuiltinType};

pub(super) fn render(model: &BindingsModel) -> String {
    let mut output = String::from(BASE);

    if model.has_builtin_type(BuiltinType::UInt8) || model.has_builtin_type(BuiltinType::Int8) {
        output.push_str(U8_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int8) {
        output.push_str(I8_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt16) || model.has_builtin_type(BuiltinType::Int16) {
        output.push_str(U16_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int16) {
        output.push_str(I16_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt32)
        || model.has_builtin_type(BuiltinType::Int32)
        || model.has_builtin_type(BuiltinType::String)
        || model.has_builtin_type(BuiltinType::Bytes)
    {
        output.push_str(U32_WIRE_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int32) {
        output.push_str(I32_CODEC);
    }
    if model.has_builtin_type(BuiltinType::UInt64) || model.has_builtin_type(BuiltinType::Int64) {
        output.push_str(U64_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Int64) {
        output.push_str(I64_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Boolean) {
        output.push_str(BOOLEAN_CODEC);
    }

    if model.has_builtin_type(BuiltinType::String) || model.has_builtin_type(BuiltinType::Bytes) {
        output.push_str(&rustbuffer_runtime(
            model.private_ffi().rustbuffer_alloc(),
            model.private_ffi().rustbuffer_free(),
        ));
    }

    if model.has_builtin_type(BuiltinType::String) {
        output.push_str(STRING_CODEC);
    }
    if model.has_builtin_type(BuiltinType::Bytes) {
        output.push_str(BYTES_CODEC);
    }

    output
}

const BASE: &str = r#"
/* Private UniFFI-compatible codecs. These symbols are not part of the C ABI. */
#if defined(__GNUC__)
#  define WALLET_ENGINE_PRIVATE __attribute__((visibility("hidden")))
#else
#  define WALLET_ENGINE_PRIVATE
#endif

typedef struct WalletEnginePrivateBufferWriter {
    uint8_t *data;
    size_t len;
    size_t offset;
} WalletEnginePrivateBufferWriter;

typedef struct WalletEnginePrivateBufferReader {
    const uint8_t *data;
    size_t len;
    size_t offset;
} WalletEnginePrivateBufferReader;

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write(
    WalletEnginePrivateBufferWriter *writer,
    const uint8_t *data,
    size_t len
) {
    if (writer == NULL || writer->offset > writer->len || len > writer->len - writer->offset) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    if (len != 0u && (writer->data == NULL || data == NULL)) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    if (len != 0u) {
        memcpy(writer->data + writer->offset, data, len);
    }
    writer->offset += len;
    return WALLET_ENGINE_ABI_STATUS_OK;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read(
    WalletEnginePrivateBufferReader *reader,
    const uint8_t **out_data,
    size_t len
) {
    if (reader == NULL || out_data == NULL || reader->offset > reader->len
        || len > reader->len - reader->offset) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    if (len != 0u && reader->data == NULL) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    *out_data = len == 0u ? NULL : reader->data + reader->offset;
    reader->offset += len;
    return WALLET_ENGINE_ABI_STATUS_OK;
}
"#;

const U8_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_u8(
    WalletEnginePrivateBufferWriter *writer,
    uint8_t value
) {
    return wallet_engine_private_write(writer, &value, sizeof(value));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_u8(
    WalletEnginePrivateBufferReader *reader,
    uint8_t *out_value
) {
    const uint8_t *data = NULL;
    WalletEngineAbiStatus status = wallet_engine_private_read(reader, &data, 1u);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        *out_value = data[0];
    }
    return status;
}
";

const I8_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_i8(
    WalletEnginePrivateBufferWriter *writer,
    int8_t value
) {
    uint8_t bits = 0u;
    memcpy(&bits, &value, sizeof(bits));
    return wallet_engine_private_write(writer, &bits, sizeof(bits));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_i8(
    WalletEnginePrivateBufferReader *reader,
    int8_t *out_value
) {
    uint8_t bits = 0u;
    WalletEngineAbiStatus status = wallet_engine_private_read_u8(reader, &bits);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        memcpy(out_value, &bits, sizeof(bits));
    }
    return status;
}
";

const U16_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_u16(
    WalletEnginePrivateBufferWriter *writer,
    uint16_t value
) {
    const uint8_t bytes[2] = {
        (uint8_t)(value >> 8u),
        (uint8_t)value,
    };
    return wallet_engine_private_write(writer, bytes, sizeof(bytes));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_u16(
    WalletEnginePrivateBufferReader *reader,
    uint16_t *out_value
) {
    const uint8_t *bytes = NULL;
    WalletEngineAbiStatus status = wallet_engine_private_read(reader, &bytes, 2u);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        *out_value = ((uint16_t)bytes[0] << 8u) | (uint16_t)bytes[1];
    }
    return status;
}
";

const I16_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_i16(
    WalletEnginePrivateBufferWriter *writer,
    int16_t value
) {
    uint16_t bits = 0u;
    memcpy(&bits, &value, sizeof(bits));
    return wallet_engine_private_write_u16(writer, bits);
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_i16(
    WalletEnginePrivateBufferReader *reader,
    int16_t *out_value
) {
    uint16_t bits = 0u;
    WalletEngineAbiStatus status = wallet_engine_private_read_u16(reader, &bits);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        memcpy(out_value, &bits, sizeof(bits));
    }
    return status;
}
";

const U32_WIRE_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_u32(
    WalletEnginePrivateBufferWriter *writer,
    uint32_t value
) {
    const uint8_t bytes[4] = {
        (uint8_t)(value >> 24u),
        (uint8_t)(value >> 16u),
        (uint8_t)(value >> 8u),
        (uint8_t)value,
    };
    return wallet_engine_private_write(writer, bytes, sizeof(bytes));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_u32(
    WalletEnginePrivateBufferReader *reader,
    uint32_t *out_value
) {
    const uint8_t *bytes = NULL;
    WalletEngineAbiStatus status = wallet_engine_private_read(reader, &bytes, 4u);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        *out_value = ((uint32_t)bytes[0] << 24u)
            | ((uint32_t)bytes[1] << 16u)
            | ((uint32_t)bytes[2] << 8u)
            | (uint32_t)bytes[3];
    }
    return status;
}
";

const I32_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_i32(
    WalletEnginePrivateBufferWriter *writer,
    int32_t value
) {
    uint32_t bits = 0u;
    memcpy(&bits, &value, sizeof(bits));
    return wallet_engine_private_write_u32(writer, bits);
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_i32(
    WalletEnginePrivateBufferReader *reader,
    int32_t *out_value
) {
    uint32_t bits = 0u;
    WalletEngineAbiStatus status = wallet_engine_private_read_u32(reader, &bits);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        memcpy(out_value, &bits, sizeof(bits));
    }
    return status;
}
";

const U64_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_u64(
    WalletEnginePrivateBufferWriter *writer,
    uint64_t value
) {
    const uint8_t bytes[8] = {
        (uint8_t)(value >> 56u),
        (uint8_t)(value >> 48u),
        (uint8_t)(value >> 40u),
        (uint8_t)(value >> 32u),
        (uint8_t)(value >> 24u),
        (uint8_t)(value >> 16u),
        (uint8_t)(value >> 8u),
        (uint8_t)value,
    };
    return wallet_engine_private_write(writer, bytes, sizeof(bytes));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_u64(
    WalletEnginePrivateBufferReader *reader,
    uint64_t *out_value
) {
    const uint8_t *bytes = NULL;
    WalletEngineAbiStatus status = wallet_engine_private_read(reader, &bytes, 8u);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        *out_value = ((uint64_t)bytes[0] << 56u)
            | ((uint64_t)bytes[1] << 48u)
            | ((uint64_t)bytes[2] << 40u)
            | ((uint64_t)bytes[3] << 32u)
            | ((uint64_t)bytes[4] << 24u)
            | ((uint64_t)bytes[5] << 16u)
            | ((uint64_t)bytes[6] << 8u)
            | (uint64_t)bytes[7];
    }
    return status;
}
";

const I64_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_i64(
    WalletEnginePrivateBufferWriter *writer,
    int64_t value
) {
    uint64_t bits = 0u;
    memcpy(&bits, &value, sizeof(bits));
    return wallet_engine_private_write_u64(writer, bits);
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_i64(
    WalletEnginePrivateBufferReader *reader,
    int64_t *out_value
) {
    uint64_t bits = 0u;
    WalletEngineAbiStatus status = wallet_engine_private_read_u64(reader, &bits);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        memcpy(out_value, &bits, sizeof(bits));
    }
    return status;
}
";

const BOOLEAN_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_bool(
    WalletEnginePrivateBufferWriter *writer,
    bool value
) {
    const uint8_t lowered = value ? 1u : 0u;
    return wallet_engine_private_write(writer, &lowered, sizeof(lowered));
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_bool(
    WalletEnginePrivateBufferReader *reader,
    bool *out_value
) {
    const uint8_t *data = NULL;
    WalletEngineAbiStatus status = wallet_engine_private_read(reader, &data, 1u);
    if (status == WALLET_ENGINE_ABI_STATUS_OK) {
        *out_value = data[0] != 0u;
    }
    return status;
}
";

fn rustbuffer_runtime(alloc_symbol: &str, free_symbol: &str) -> String {
    format!(
        r"
typedef struct WalletEnginePrivateRustBuffer {{
    uint64_t capacity;
    uint64_t len;
    uint8_t *data;
}} WalletEnginePrivateRustBuffer;

typedef struct WalletEnginePrivateRustCallStatus {{
    int8_t code;
    WalletEnginePrivateRustBuffer error_buf;
}} WalletEnginePrivateRustCallStatus;

extern WalletEnginePrivateRustBuffer {alloc_symbol}(
    uint64_t size,
    WalletEnginePrivateRustCallStatus *out_status
);

extern void {free_symbol}(
    WalletEnginePrivateRustBuffer buffer,
    WalletEnginePrivateRustCallStatus *out_status
);

WALLET_ENGINE_PRIVATE bool wallet_engine_private_rustbuffer_is_valid(
    const WalletEnginePrivateRustBuffer *buffer
) {{
    if (buffer == NULL || buffer->len > buffer->capacity) {{
        return false;
    }}
    if (buffer->data == NULL) {{
        return buffer->len == 0u && buffer->capacity == 0u;
    }}
    return (uint64_t)(size_t)buffer->len == buffer->len;
}}

WALLET_ENGINE_PRIVATE void wallet_engine_private_rustbuffer_free(
    WalletEnginePrivateRustBuffer buffer
) {{
    WalletEnginePrivateRustCallStatus status = {{0}};
    if (buffer.data != NULL || buffer.len != 0u || buffer.capacity != 0u) {{
        {free_symbol}(buffer, &status);
    }}
    if (status.error_buf.data != NULL || status.error_buf.len != 0u
        || status.error_buf.capacity != 0u) {{
        WalletEnginePrivateRustCallStatus ignored = {{0}};
        {free_symbol}(status.error_buf, &ignored);
    }}
}}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_rustbuffer_alloc(
    size_t size,
    WalletEnginePrivateRustBuffer *out_buffer
) {{
    WalletEnginePrivateRustCallStatus status = {{0}};
    WalletEnginePrivateRustBuffer buffer = {{0}};
    const uint64_t wire_size = (uint64_t)size;
    if (out_buffer == NULL || (size_t)wire_size != size) {{
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }}

    *out_buffer = buffer;
    buffer = {alloc_symbol}(wire_size, &status);
    if (status.code != 0) {{
        wallet_engine_private_rustbuffer_free(status.error_buf);
        wallet_engine_private_rustbuffer_free(buffer);
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }}
    if (!wallet_engine_private_rustbuffer_is_valid(&buffer) || buffer.len != wire_size) {{
        wallet_engine_private_rustbuffer_free(buffer);
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }}

    *out_buffer = buffer;
    return WALLET_ENGINE_ABI_STATUS_OK;
}}
"
    )
}

const STRING_CODEC: &str = r"
WALLET_ENGINE_PRIVATE bool wallet_engine_private_is_utf8(
    const uint8_t *data,
    size_t len
) {
    size_t index = 0u;
    if (len != 0u && data == NULL) {
        return false;
    }
    while (index < len) {
        const uint8_t first = data[index];
        if (first <= 0x7fu) {
            index += 1u;
        } else if (first >= 0xc2u && first <= 0xdfu) {
            if (len - index < 2u || (data[index + 1u] & 0xc0u) != 0x80u) {
                return false;
            }
            index += 2u;
        } else if (first >= 0xe0u && first <= 0xefu) {
            if (len - index < 3u || (data[index + 1u] & 0xc0u) != 0x80u
                || (data[index + 2u] & 0xc0u) != 0x80u
                || (first == 0xe0u && data[index + 1u] < 0xa0u)
                || (first == 0xedu && data[index + 1u] >= 0xa0u)) {
                return false;
            }
            index += 3u;
        } else if (first >= 0xf0u && first <= 0xf4u) {
            if (len - index < 4u || (data[index + 1u] & 0xc0u) != 0x80u
                || (data[index + 2u] & 0xc0u) != 0x80u
                || (data[index + 3u] & 0xc0u) != 0x80u
                || (first == 0xf0u && data[index + 1u] < 0x90u)
                || (first == 0xf4u && data[index + 1u] >= 0x90u)) {
                return false;
            }
            index += 4u;
        } else {
            return false;
        }
    }
    return true;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_string(
    WalletEnginePrivateBufferWriter *writer,
    WalletEngineStringView value
) {
    WalletEngineAbiStatus status;
    if ((value.len != 0u && value.data == NULL) || value.len > (size_t)INT32_MAX) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    if (!wallet_engine_private_is_utf8((const uint8_t *)value.data, value.len)) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_UTF8;
    }
    status = wallet_engine_private_write_u32(writer, (uint32_t)value.len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        return status;
    }
    return wallet_engine_private_write(writer, (const uint8_t *)value.data, value.len);
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_string(
    WalletEnginePrivateBufferReader *reader,
    WalletEngineStringView *out_value
) {
    uint32_t len = 0u;
    const uint8_t *data = NULL;
    WalletEngineAbiStatus status;
    if (out_value == NULL) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_read_u32(reader, &len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK || len > (uint32_t)INT32_MAX) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_read(reader, &data, (size_t)len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        return status;
    }
    if (!wallet_engine_private_is_utf8(data, (size_t)len)) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_UTF8;
    }
    out_value->data = (const char *)data;
    out_value->len = (size_t)len;
    return WALLET_ENGINE_ABI_STATUS_OK;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_lower_string(
    WalletEngineStringView value,
    WalletEnginePrivateRustBuffer *out_buffer
) {
    WalletEngineAbiStatus status;
    if ((value.len != 0u && value.data == NULL)
        || !wallet_engine_private_is_utf8((const uint8_t *)value.data, value.len)) {
        return value.len != 0u && value.data == NULL
            ? WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT
            : WALLET_ENGINE_ABI_STATUS_INVALID_UTF8;
    }
    status = wallet_engine_private_rustbuffer_alloc(value.len, out_buffer);
    if (status == WALLET_ENGINE_ABI_STATUS_OK && value.len != 0u) {
        memcpy(out_buffer->data, value.data, value.len);
    }
    return status;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_lift_string(
    const WalletEnginePrivateRustBuffer *buffer,
    WalletEngineStringView *out_value
) {
    size_t len;
    if (out_value == NULL || !wallet_engine_private_rustbuffer_is_valid(buffer)) {
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }
    len = (size_t)buffer->len;
    if (!wallet_engine_private_is_utf8(buffer->data, len)) {
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }
    out_value->data = (const char *)buffer->data;
    out_value->len = len;
    return WALLET_ENGINE_ABI_STATUS_OK;
}
";

const BYTES_CODEC: &str = r"
WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_write_bytes(
    WalletEnginePrivateBufferWriter *writer,
    WalletEngineBytesView value
) {
    WalletEngineAbiStatus status;
    if ((value.len != 0u && value.data == NULL) || value.len > (size_t)INT32_MAX) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_write_u32(writer, (uint32_t)value.len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        return status;
    }
    return wallet_engine_private_write(writer, value.data, value.len);
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_read_bytes(
    WalletEnginePrivateBufferReader *reader,
    WalletEngineBytesView *out_value
) {
    uint32_t len = 0u;
    const uint8_t *data = NULL;
    WalletEngineAbiStatus status;
    if (out_value == NULL) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_read_u32(reader, &len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK || len > (uint32_t)INT32_MAX) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_read(reader, &data, (size_t)len);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        return status;
    }
    out_value->data = data;
    out_value->len = (size_t)len;
    return WALLET_ENGINE_ABI_STATUS_OK;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_lower_bytes(
    WalletEngineBytesView value,
    WalletEnginePrivateRustBuffer *out_buffer
) {
    WalletEnginePrivateBufferWriter writer;
    WalletEngineAbiStatus status;
    if ((value.len != 0u && value.data == NULL) || value.len > (size_t)INT32_MAX
        || value.len > SIZE_MAX - 4u) {
        return WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT;
    }
    status = wallet_engine_private_rustbuffer_alloc(value.len + 4u, out_buffer);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        return status;
    }
    writer.data = out_buffer->data;
    writer.len = (size_t)out_buffer->len;
    writer.offset = 0u;
    status = wallet_engine_private_write_bytes(&writer, value);
    if (status != WALLET_ENGINE_ABI_STATUS_OK) {
        wallet_engine_private_rustbuffer_free(*out_buffer);
        *out_buffer = (WalletEnginePrivateRustBuffer){0};
    }
    return status;
}

WALLET_ENGINE_PRIVATE WalletEngineAbiStatus wallet_engine_private_lift_bytes(
    const WalletEnginePrivateRustBuffer *buffer,
    WalletEngineBytesView *out_value
) {
    WalletEnginePrivateBufferReader reader;
    WalletEngineAbiStatus status;
    if (out_value == NULL || !wallet_engine_private_rustbuffer_is_valid(buffer)) {
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }
    reader.data = buffer->data;
    reader.len = (size_t)buffer->len;
    reader.offset = 0u;
    status = wallet_engine_private_read_bytes(&reader, out_value);
    if (status != WALLET_ENGINE_ABI_STATUS_OK || reader.offset != reader.len) {
        return WALLET_ENGINE_ABI_STATUS_PANIC;
    }
    return WALLET_ENGINE_ABI_STATUS_OK;
}
";

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use super::render;
    use crate::model::BindingsModel;

    #[test]
    fn renders_only_reachable_public_builtin_codecs() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r"
            namespace wallet_engine {};

            dictionary Example {
                u16 count;
                string name;
            };
            ",
            "wallet_engine",
        )?;
        let model = BindingsModel::from_components(&[component])?;
        let codec = render(&model);

        assert!(codec.contains("wallet_engine_private_write_u16"));
        assert!(codec.contains("wallet_engine_private_lower_string"));
        assert!(codec.contains("ffi_wallet_engine_rustbuffer_alloc"));
        assert!(!codec.contains("wallet_engine_private_write_i64"));
        Ok(())
    }
}
