#include <assert.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>

/* The codecs are private implementation details, so test them in their TU. */
#include "wallet_engine.c"

static size_t live_allocations = 0u;

WalletEnginePrivateRustBuffer ffi_wallet_engine_rustbuffer_alloc(
    uint64_t size,
    WalletEnginePrivateRustCallStatus *out_status
) {
    WalletEnginePrivateRustBuffer buffer = {0};
    assert(out_status != NULL);
    out_status->code = 0;
    out_status->error_buf = (WalletEnginePrivateRustBuffer){0};
    if (size != 0u) {
        assert(size <= (uint64_t)SIZE_MAX);
        buffer.data = malloc((size_t)size);
        if (buffer.data == NULL) {
            out_status->code = 2;
            return buffer;
        }
        live_allocations += 1u;
    }
    buffer.capacity = size;
    buffer.len = size;
    return buffer;
}

void ffi_wallet_engine_rustbuffer_free(
    WalletEnginePrivateRustBuffer buffer,
    WalletEnginePrivateRustCallStatus *out_status
) {
    assert(out_status != NULL);
    if (buffer.data != NULL) {
        assert(live_allocations != 0u);
        live_allocations -= 1u;
    }
    free(buffer.data);
    out_status->code = 0;
    out_status->error_buf = (WalletEnginePrivateRustBuffer){0};
}

static void assert_bytes(
    const uint8_t *actual,
    const uint8_t *expected,
    size_t len
) {
    assert(len == 0u || (actual != NULL && expected != NULL));
    assert(len == 0u || memcmp(actual, expected, len) == 0);
}

static void test_u8_round_trip(void) {
    static const uint8_t expected[] = {0xa5u};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    uint8_t actual = 0u;

    assert(wallet_engine_private_write_u8(&writer, UINT8_C(0xa5))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(writer.offset == sizeof(expected));
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_u8(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == UINT8_C(0xa5));
    assert(reader.offset == sizeof(expected));
}

static void test_i8_round_trip(void) {
    static const uint8_t expected[] = {0xfeu};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    int8_t actual = 0;

    assert(wallet_engine_private_write_i8(&writer, INT8_C(-2))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_i8(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == INT8_C(-2));
    assert(reader.offset == sizeof(expected));
}

static void test_u16_round_trip(void) {
    static const uint8_t expected[] = {0x12u, 0x34u};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    uint16_t actual = 0u;

    assert(wallet_engine_private_write_u16(&writer, UINT16_C(0x1234))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_u16(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == UINT16_C(0x1234));
    assert(reader.offset == sizeof(expected));
}

static void test_i16_round_trip(void) {
    static const uint8_t expected[] = {0xffu, 0xfeu};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    int16_t actual = 0;

    assert(wallet_engine_private_write_i16(&writer, INT16_C(-2))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_i16(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == INT16_C(-2));
    assert(reader.offset == sizeof(expected));
}

static void test_u32_round_trip(void) {
    static const uint8_t expected[] = {0x12u, 0x34u, 0x56u, 0x78u};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    uint32_t actual = 0u;

    assert(wallet_engine_private_write_u32(&writer, UINT32_C(0x12345678))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_u32(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == UINT32_C(0x12345678));
    assert(reader.offset == sizeof(expected));
}

static void test_i32_round_trip(void) {
    static const uint8_t expected[] = {0xffu, 0xffu, 0xffu, 0xfeu};
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    int32_t actual = 0;

    assert(wallet_engine_private_write_i32(&writer, INT32_C(-2))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_i32(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == INT32_C(-2));
    assert(reader.offset == sizeof(expected));
}

static void test_u64_round_trip(void) {
    static const uint8_t expected[] = {
        0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u,
    };
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    uint64_t actual = 0u;

    assert(wallet_engine_private_write_u64(&writer, UINT64_C(0x0102030405060708))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_u64(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == UINT64_C(0x0102030405060708));
    assert(reader.offset == sizeof(expected));
}

static void test_i64_round_trip(void) {
    static const uint8_t expected[] = {
        0xffu, 0xffu, 0xffu, 0xffu, 0xffu, 0xffu, 0xffu, 0xfeu,
    };
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    int64_t actual = 0;

    assert(wallet_engine_private_write_i64(&writer, INT64_C(-2))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_i64(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == INT64_C(-2));
    assert(reader.offset == sizeof(expected));
}

static void test_bool_round_trip(void) {
    uint8_t wire[] = {0xffu};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    bool actual = false;

    assert(wallet_engine_private_write_bool(&writer, true)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(wire[0] == 1u);

    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_bool(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual);
    assert(reader.offset == sizeof(wire));

    wire[0] = 0x7fu;
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    actual = false;
    assert(wallet_engine_private_read_bool(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual);
}

static void test_flat_enum_round_trip(void) {
    static const uint8_t mainnet_wire[] = {0x00u, 0x00u, 0x00u, 0x01u};
    static const uint8_t testnet_wire[] = {0x00u, 0x00u, 0x00u, 0x02u};
    static const uint8_t unknown_wire[] = {0x00u, 0x00u, 0x00u, 0x03u};
    uint8_t wire[4] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    WalletEngineNetwork actual = UINT32_MAX;
    WalletEnginePrivateRustBuffer buffer = {0};

    assert(wallet_engine_private_write_network(&writer, WALLET_ENGINE_NETWORK_MAINNET)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, mainnet_wire, sizeof(mainnet_wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_network(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == WALLET_ENGINE_NETWORK_MAINNET);

    writer = (WalletEnginePrivateBufferWriter){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_write_network(&writer, WALLET_ENGINE_NETWORK_TESTNET)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, testnet_wire, sizeof(testnet_wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_network(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == WALLET_ENGINE_NETWORK_TESTNET);

    assert(wallet_engine_private_lower_network(WALLET_ENGINE_NETWORK_TESTNET, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(testnet_wire));
    assert_bytes(buffer.data, testnet_wire, sizeof(testnet_wire));
    actual = UINT32_MAX;
    assert(wallet_engine_private_lift_network(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual == WALLET_ENGINE_NETWORK_TESTNET);
    wallet_engine_private_rustbuffer_free(buffer);
    assert(live_allocations == 0u);

    writer = (WalletEnginePrivateBufferWriter){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_write_network(&writer, (WalletEngineNetwork)2u)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(writer.offset == 0u);
    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_network((WalletEngineNetwork)2u, &buffer)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    assert(live_allocations == 0u);

    reader = (WalletEnginePrivateBufferReader){unknown_wire, sizeof(unknown_wire), 0u};
    assert(wallet_engine_private_read_network(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(reader.offset == sizeof(unknown_wire));
    reader = (WalletEnginePrivateBufferReader){mainnet_wire, sizeof(mainnet_wire), 0u};
    assert(wallet_engine_private_read_network(&reader, NULL)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(reader.offset == 0u);
}

static void test_string_round_trip(void) {
    static const char text[] = {
        'T', 'O', 'N', ' ', (char)0xf0, (char)0x9f, (char)0x92, (char)0x8e,
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x08u,
        'T', 'O', 'N', ' ', 0xf0u, 0x9fu, 0x92u, 0x8eu,
    };
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    WalletEngineStringView input = {text, sizeof(text)};
    WalletEngineStringView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};

    assert(wallet_engine_private_write_string(&writer, input)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_string(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == input.len);
    assert_bytes((const uint8_t *)actual.data, (const uint8_t *)input.data, input.len);
    assert(reader.offset == sizeof(wire));

    assert(wallet_engine_private_lower_string(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == input.len);
    assert_bytes(buffer.data, (const uint8_t *)input.data, input.len);
    actual = (WalletEngineStringView){0};
    assert(wallet_engine_private_lift_string(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == input.len);
    assert_bytes((const uint8_t *)actual.data, (const uint8_t *)input.data, input.len);
    wallet_engine_private_rustbuffer_free(buffer);
    assert(live_allocations == 0u);
}

static void test_empty_string_round_trip(void) {
    uint8_t wire[4] = {0xffu, 0xffu, 0xffu, 0xffu};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    WalletEngineStringView actual = {(const char *)UINTPTR_MAX, SIZE_MAX};

    assert(wallet_engine_private_write_string(&writer, (WalletEngineStringView){NULL, 0u})
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, (const uint8_t[4]){0u, 0u, 0u, 0u}, sizeof(wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_string(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.data == NULL);
    assert(actual.len == 0u);
}

static void test_bytes_round_trip(void) {
    static const uint8_t payload[] = {0x00u, 0x01u, 0x7fu, 0x80u, 0xffu};
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x05u, 0x00u, 0x01u, 0x7fu, 0x80u, 0xffu,
    };
    uint8_t wire[sizeof(expected)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    WalletEngineBytesView input = {payload, sizeof(payload)};
    WalletEngineBytesView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};

    assert(wallet_engine_private_write_bytes(&writer, input)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, expected, sizeof(expected));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_bytes(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == input.len);
    assert_bytes(actual.data, input.data, input.len);
    assert(reader.offset == sizeof(wire));

    assert(wallet_engine_private_lower_bytes(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    actual = (WalletEngineBytesView){0};
    assert(wallet_engine_private_lift_bytes(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == input.len);
    assert_bytes(actual.data, input.data, input.len);
    wallet_engine_private_rustbuffer_free(buffer);
    assert(live_allocations == 0u);
}

static void test_bounds_failures(void) {
    static const uint8_t one_byte[] = {0u};
    uint8_t wire[3] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader = {wire, sizeof(wire), 0u};
    const uint8_t *read = NULL;
    uint32_t integer = 0u;

    assert(wallet_engine_private_write(&writer, one_byte, sizeof(one_byte))
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(wallet_engine_private_write_u32(&writer, UINT32_C(1))
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(writer.offset == 1u);
    assert(wallet_engine_private_write(NULL, one_byte, sizeof(one_byte))
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(wallet_engine_private_write(&writer, NULL, sizeof(one_byte))
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);

    assert(wallet_engine_private_read_u32(&reader, &integer)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(reader.offset == 0u);
    assert(wallet_engine_private_read(NULL, &read, 0u)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    reader = (WalletEnginePrivateBufferReader){NULL, 1u, 0u};
    assert(wallet_engine_private_read(&reader, &read, 1u)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
}

static void test_malformed_strings(void) {
    static const uint8_t invalid[][4] = {
        {0xc0u, 0x80u, 0u, 0u},
        {0xedu, 0xa0u, 0x80u, 0u},
        {0xf4u, 0x90u, 0x80u, 0x80u},
        {0xf0u, 0x9fu, 0x92u, 0u},
    };
    static const size_t invalid_lengths[] = {2u, 3u, 4u, 3u};
    size_t index;

    for (index = 0u; index < sizeof(invalid_lengths) / sizeof(invalid_lengths[0]); index += 1u) {
        WalletEnginePrivateRustBuffer buffer = {0};
        WalletEngineStringView value = {(const char *)invalid[index], invalid_lengths[index]};
        assert(wallet_engine_private_lower_string(value, &buffer)
            == WALLET_ENGINE_ABI_STATUS_INVALID_UTF8);
        assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    }
    assert(wallet_engine_private_lower_string(
        (WalletEngineStringView){NULL, 1u},
        &(WalletEnginePrivateRustBuffer){0}
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(live_allocations == 0u);
}

static void test_malformed_compound_values(void) {
    static uint8_t negative_length[] = {0x80u, 0x00u, 0x00u, 0x00u};
    static uint8_t truncated[] = {0x00u, 0x00u, 0x00u, 0x02u, 0xaau};
    static uint8_t trailing[] = {0x00u, 0x00u, 0x00u, 0x01u, 0xaau, 0xbbu};
    WalletEnginePrivateBufferReader reader;
    WalletEngineBytesView bytes = {0};
    WalletEngineStringView string = {0};
    WalletEnginePrivateRustBuffer buffer;

    reader = (WalletEnginePrivateBufferReader){negative_length, sizeof(negative_length), 0u};
    assert(wallet_engine_private_read_bytes(&reader, &bytes)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    reader = (WalletEnginePrivateBufferReader){negative_length, sizeof(negative_length), 0u};
    assert(wallet_engine_private_read_string(&reader, &string)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);

    buffer = (WalletEnginePrivateRustBuffer){sizeof(truncated), sizeof(truncated), truncated};
    assert(wallet_engine_private_lift_bytes(&buffer, &bytes)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_bytes(&buffer, &bytes)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){0u, 1u, NULL};
    assert(wallet_engine_private_lift_string(&buffer, &string)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
}

int main(void) {
    test_u8_round_trip();
    test_i8_round_trip();
    test_u16_round_trip();
    test_i16_round_trip();
    test_u32_round_trip();
    test_i32_round_trip();
    test_u64_round_trip();
    test_i64_round_trip();
    test_bool_round_trip();
    test_flat_enum_round_trip();
    test_string_round_trip();
    test_empty_string_round_trip();
    test_bytes_round_trip();
    test_bounds_failures();
    test_malformed_strings();
    test_malformed_compound_values();
    assert(live_allocations == 0u);
    return EXIT_SUCCESS;
}
