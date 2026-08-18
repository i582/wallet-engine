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

static void test_optional_u64_round_trip(void) {
    static const uint8_t none_wire[] = {0x00u};
    static const uint8_t some_wire[] = {
        0x01u, 0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u,
    };
    static const uint8_t unknown_tag[] = {0x02u};
    static uint8_t truncated[] = {0x01u};
    static uint8_t trailing[] = {
        0x01u, 0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u, 0xffu,
    };
    uint8_t wire[sizeof(some_wire)] = {0};
    WalletEnginePrivateBufferWriter writer;
    WalletEnginePrivateBufferReader reader;
    WalletEngineOptionalU64 actual = {true, UINT64_MAX};
    WalletEnginePrivateRustBuffer buffer = {0};

    writer = (WalletEnginePrivateBufferWriter){wire, sizeof(none_wire), 0u};
    assert(wallet_engine_private_write_optional_u64(
        &writer,
        (WalletEngineOptionalU64){false, UINT64_MAX}
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, none_wire, sizeof(none_wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(none_wire), 0u};
    assert(wallet_engine_private_read_optional_u64(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(!actual.has_value && actual.value == 0u);

    writer = (WalletEnginePrivateBufferWriter){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_write_optional_u64(
        &writer,
        (WalletEngineOptionalU64){true, UINT64_C(0x0102030405060708)}
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, some_wire, sizeof(some_wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_optional_u64(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.has_value && actual.value == UINT64_C(0x0102030405060708));

    assert(wallet_engine_private_lower_optional_u64(
        (WalletEngineOptionalU64){true, UINT64_C(0x0102030405060708)},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(some_wire));
    assert_bytes(buffer.data, some_wire, sizeof(some_wire));
    actual = (WalletEngineOptionalU64){0};
    assert(wallet_engine_private_lift_optional_u64(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.has_value && actual.value == UINT64_C(0x0102030405060708));
    wallet_engine_private_rustbuffer_free(buffer);

    assert(wallet_engine_private_lower_optional_u64(
        (WalletEngineOptionalU64){false, UINT64_MAX},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(none_wire));
    assert_bytes(buffer.data, none_wire, sizeof(none_wire));
    wallet_engine_private_rustbuffer_free(buffer);
    assert(live_allocations == 0u);

    reader = (WalletEnginePrivateBufferReader){unknown_tag, sizeof(unknown_tag), 0u};
    assert(wallet_engine_private_read_optional_u64(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(truncated), sizeof(truncated), truncated};
    assert(wallet_engine_private_lift_optional_u64(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_optional_u64(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
}

static void test_optional_string_round_trip(void) {
    static const char text[] = {'T', 'O', 'N'};
    static const uint8_t empty_wire[] = {0x01u, 0x00u, 0x00u, 0x00u, 0x00u};
    static const uint8_t text_wire[] = {
        0x01u, 0x00u, 0x00u, 0x00u, 0x03u, 'T', 'O', 'N',
    };
    static const uint8_t invalid_utf8[] = {0xc0u, 0x80u};
    uint8_t wire[sizeof(empty_wire)] = {0};
    WalletEnginePrivateBufferWriter writer = {wire, sizeof(wire), 0u};
    WalletEnginePrivateBufferReader reader;
    WalletEngineOptionalStringView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};

    assert(wallet_engine_private_write_optional_string(
        &writer,
        (WalletEngineOptionalStringView){true, {NULL, 0u}}
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert_bytes(wire, empty_wire, sizeof(empty_wire));
    reader = (WalletEnginePrivateBufferReader){wire, sizeof(wire), 0u};
    assert(wallet_engine_private_read_optional_string(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.has_value && actual.value.data == NULL && actual.value.len == 0u);

    assert(wallet_engine_private_lower_optional_string(
        (WalletEngineOptionalStringView){true, {text, sizeof(text)}},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(text_wire));
    assert_bytes(buffer.data, text_wire, sizeof(text_wire));
    actual = (WalletEngineOptionalStringView){0};
    assert(wallet_engine_private_lift_optional_string(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.has_value && actual.value.len == sizeof(text));
    assert_bytes(
        (const uint8_t *)actual.value.data,
        (const uint8_t *)text,
        sizeof(text)
    );
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_optional_string(
        (WalletEngineOptionalStringView){
            true,
            {(const char *)invalid_utf8, sizeof(invalid_utf8)},
        },
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_UTF8);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    assert(live_allocations == 0u);
}

static void test_optional_flat_enum_round_trip(void) {
    static const uint8_t testnet_wire[] = {0x01u, 0x00u, 0x00u, 0x00u, 0x02u};
    static const uint8_t unknown_enum[] = {0x01u, 0x00u, 0x00u, 0x00u, 0x03u};
    WalletEngineOptionalNetwork actual = {0};
    WalletEnginePrivateBufferReader reader;
    WalletEnginePrivateRustBuffer buffer = {0};

    assert(wallet_engine_private_lower_optional_network(
        (WalletEngineOptionalNetwork){true, WALLET_ENGINE_NETWORK_TESTNET},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(testnet_wire));
    assert_bytes(buffer.data, testnet_wire, sizeof(testnet_wire));
    assert(wallet_engine_private_lift_optional_network(&buffer, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.has_value && actual.value == WALLET_ENGINE_NETWORK_TESTNET);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_optional_network(
        (WalletEngineOptionalNetwork){true, (WalletEngineNetwork)2u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    reader = (WalletEnginePrivateBufferReader){unknown_enum, sizeof(unknown_enum), 0u};
    assert(wallet_engine_private_read_optional_network(&reader, &actual)
        == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(live_allocations == 0u);
}

static void test_sequence_u64_round_trip(void) {
    static const uint64_t values[] = {
        UINT64_C(0x0102030405060708),
        UINT64_C(0),
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u,
        0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u,
    };
    static uint8_t negative_count[] = {0xffu, 0xffu, 0xffu, 0xffu};
    static uint8_t truncated[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x01u,
    };
    static uint8_t impossible_count[] = {0x7fu, 0xffu, 0xffu, 0xffu};
    static uint8_t trailing[] = {
        0x00u, 0x00u, 0x00u, 0x00u, 0xffu,
    };
    WalletEngineU64ListView input = {values, 2u};
    WalletEngineU64ListView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_sequence_u64(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == input.len && actual.data != NULL);
    assert(actual.data[0] == values[0] && actual.data[1] == values[1]);
    assert(arena.head != NULL);
    wallet_engine_private_arena_clear(&arena);
    assert(arena.head == NULL);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_sequence_u64(
        (WalletEngineU64ListView){NULL, 0u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == 4u);
    assert_bytes(buffer.data, (const uint8_t[4]){0u, 0u, 0u, 0u}, 4u);
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_sequence_u64(
        (WalletEngineU64ListView){NULL, 1u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);

    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(negative_count), sizeof(negative_count), negative_count,
    };
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(truncated), sizeof(truncated), truncated};
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(impossible_count), sizeof(impossible_count), impossible_count,
    };
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_sequence_u64(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    assert(live_allocations == 0u);
}

static void test_sequence_string_round_trip(void) {
    static const char text[] = {'T', 'O', 'N'};
    static const WalletEngineStringView values[] = {
        {text, sizeof(text)},
        {NULL, 0u},
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x03u, 'T', 'O', 'N',
        0x00u, 0x00u, 0x00u, 0x00u,
    };
    static const uint8_t invalid_utf8[] = {0xc0u, 0x80u};
    static uint8_t invalid_utf8_wire[] = {
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x02u, 0xc0u, 0x80u,
    };
    WalletEngineStringListView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_sequence_string(
        (WalletEngineStringListView){values, 2u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_sequence_string(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == 2u && actual.data != NULL);
    assert(actual.data[0].len == sizeof(text));
    assert_bytes(
        (const uint8_t *)actual.data[0].data,
        (const uint8_t *)text,
        sizeof(text)
    );
    assert(actual.data[1].data == NULL && actual.data[1].len == 0u);
    wallet_engine_private_arena_clear(&arena);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_sequence_string(
        (WalletEngineStringListView){
            (const WalletEngineStringView[]){
                {(const char *)invalid_utf8, sizeof(invalid_utf8)},
            },
            1u,
        },
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_UTF8);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(invalid_utf8_wire), sizeof(invalid_utf8_wire), invalid_utf8_wire,
    };
    assert(wallet_engine_private_lift_sequence_string(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    assert(live_allocations == 0u);
}

static void test_sequence_flat_enum_round_trip(void) {
    static const WalletEngineNetwork values[] = {
        WALLET_ENGINE_NETWORK_MAINNET,
        WALLET_ENGINE_NETWORK_TESTNET,
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x02u,
    };
    static uint8_t unknown_enum[] = {
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x03u,
    };
    WalletEngineNetworkListView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_sequence_network(
        (WalletEngineNetworkListView){values, 2u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_sequence_network(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.len == 2u && actual.data != NULL);
    assert(actual.data[0] == values[0] && actual.data[1] == values[1]);
    wallet_engine_private_arena_clear(&arena);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_sequence_network(
        (WalletEngineNetworkListView){
            (const WalletEngineNetwork[]){(WalletEngineNetwork)2u},
            1u,
        },
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(unknown_enum), sizeof(unknown_enum), unknown_enum,
    };
    assert(wallet_engine_private_lift_sequence_network(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.data == NULL && actual.len == 0u && arena.head == NULL);
    assert(live_allocations == 0u);
}

static void test_record_round_trip(void) {
    static const char label[] = {'T', 'O', 'N'};
    static const char alias[] = {'m', 'a', 'i', 'n'};
    static const WalletEngineStringView aliases[] = {
        {alias, sizeof(alias)},
        {NULL, 0u},
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x03u, 'T', 'O', 'N',
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x04u, 'm', 'a', 'i', 'n',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x02u,
        0x01u, 0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u,
    };
    static const uint8_t invalid_utf8[] = {0xc0u, 0x80u};
    static uint8_t unknown_enum[] = {
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x03u,
    };
    static uint8_t truncated_optional[] = {
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x01u,
    };
    static uint8_t trailing[] = {
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u,
        0xffu,
    };
    WalletEngineRecordFixtureView input = {
        {label, sizeof(label)},
        {aliases, 2u},
        WALLET_ENGINE_NETWORK_TESTNET,
        {true, UINT64_C(0x0102030405060708)},
    };
    WalletEngineRecordFixtureView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_record_fixture(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.label.len == sizeof(label));
    assert_bytes(
        (const uint8_t *)actual.label.data,
        (const uint8_t *)label,
        sizeof(label)
    );
    assert(actual.aliases.len == 2u && actual.aliases.data != NULL);
    assert(actual.aliases.data[0].len == sizeof(alias));
    assert_bytes(
        (const uint8_t *)actual.aliases.data[0].data,
        (const uint8_t *)alias,
        sizeof(alias)
    );
    assert(actual.aliases.data[1].data == NULL && actual.aliases.data[1].len == 0u);
    assert(actual.network == WALLET_ENGINE_NETWORK_TESTNET);
    assert(actual.revision.has_value);
    assert(actual.revision.value == UINT64_C(0x0102030405060708));
    assert(arena.head != NULL);
    wallet_engine_private_arena_clear(&arena);
    wallet_engine_private_rustbuffer_free(buffer);

    input.label = (WalletEngineStringView){
        (const char *)invalid_utf8,
        sizeof(invalid_utf8),
    };
    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_record_fixture(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_INVALID_UTF8);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);

    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(unknown_enum), sizeof(unknown_enum), unknown_enum,
    };
    assert(wallet_engine_private_lift_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(arena.head == NULL);
    assert(actual.label.data == NULL && actual.aliases.data == NULL);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(truncated_optional), sizeof(truncated_optional), truncated_optional,
    };
    assert(wallet_engine_private_lift_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(arena.head == NULL);
    assert(actual.label.data == NULL && actual.aliases.data == NULL);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(arena.head == NULL);
    assert(actual.label.data == NULL && actual.aliases.data == NULL);
    assert(live_allocations == 0u);
}

static void test_nested_record_round_trip(void) {
    static const char label[] = {'T', 'O', 'N'};
    static const WalletEngineStringView aliases[] = {
        {label, sizeof(label)},
    };
    WalletEngineNestedRecordFixtureView input = {
        {
            {label, sizeof(label)},
            {aliases, 1u},
            WALLET_ENGINE_NETWORK_MAINNET,
            {false, UINT64_MAX},
        },
        true,
    };
    WalletEngineNestedRecordFixtureView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_nested_record_fixture(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(wallet_engine_private_lift_nested_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.enabled);
    assert(actual.value.label.len == sizeof(label));
    assert(actual.value.aliases.len == 1u);
    assert(actual.value.aliases.data != NULL);
    assert(actual.value.network == WALLET_ENGINE_NETWORK_MAINNET);
    assert(!actual.value.revision.has_value && actual.value.revision.value == 0u);
    wallet_engine_private_arena_clear(&arena);
    wallet_engine_private_rustbuffer_free(buffer);
    assert(live_allocations == 0u);
}

static void test_nested_compound_round_trip(void) {
    static const char label[] = {'A'};
    static const WalletEngineRecordFixtureView records[] = {
        {
            {label, sizeof(label)},
            {NULL, 0u},
            WALLET_ENGINE_NETWORK_MAINNET,
            {false, UINT64_MAX},
        },
    };
    static const uint8_t record_wire[] = {
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u,
    };
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u,
        0x01u,
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u,
    };
    static uint8_t impossible_sequence[4u + sizeof(record_wire)] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u,
    };
    static uint8_t truncated_optional[] = {
        0x01u,
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x00u,
        0x00u, 0x00u, 0x00u, 0x01u,
    };
    WalletEngineNestedCompoundFixtureView input = {
        {records, 1u},
        {true, records[0]},
    };
    WalletEngineNestedCompoundFixtureView actual = {0};
    WalletEngineRecordFixtureListView sequence = {0};
    WalletEngineOptionalRecordFixtureView optional = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_nested_compound_fixture(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_nested_compound_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.records.len == 1u && actual.records.data != NULL);
    assert(actual.records.data[0].label.len == sizeof(label));
    assert(actual.records.data[0].aliases.len == 0u);
    assert(actual.selected.has_value);
    assert(actual.selected.value.network == WALLET_ENGINE_NETWORK_MAINNET);
    assert(!actual.selected.value.revision.has_value);
    wallet_engine_private_arena_clear(&arena);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(impossible_sequence), sizeof(impossible_sequence), impossible_sequence,
    };
    assert(wallet_engine_private_lift_sequence_record_fixture(
        &buffer,
        &arena,
        &sequence
    ) == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(sequence.data == NULL && sequence.len == 0u && arena.head == NULL);

    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(truncated_optional), sizeof(truncated_optional), truncated_optional,
    };
    assert(wallet_engine_private_lift_optional_record_fixture(
        &buffer,
        &arena,
        &optional
    ) == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(!optional.has_value && arena.head == NULL);
    assert(live_allocations == 0u);
}

static void test_empty_record_round_trip(void) {
    static uint8_t trailing[] = {0xffu};
    WalletEngineEmptyRecordFixtureView actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_empty_record_fixture(
        (WalletEngineEmptyRecordFixtureView){0},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);
    assert(wallet_engine_private_lift_empty_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(arena.head == NULL);

    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_empty_record_fixture(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(arena.head == NULL);
    assert(live_allocations == 0u);
}

static void test_rich_error_round_trip(void) {
    static const char diagnostic[] = {'T', 'O', 'N'};
    static const uint8_t expected[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x03u, 'T', 'O', 'N',
        0x01u, 0x01u, 0x02u, 0x03u, 0x04u, 0x05u, 0x06u, 0x07u, 0x08u,
    };
    static uint8_t unknown_tag[] = {0x00u, 0x00u, 0x00u, 0x04u};
    static uint8_t invalid_network[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x03u,
    };
    static uint8_t truncated_diagnostic[] = {
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x01u,
        0x00u, 0x00u, 0x00u, 0x03u, 'N', 'O',
    };
    static uint8_t trailing[] = {0x00u, 0x00u, 0x00u, 0x01u, 0xffu};
    static uint8_t malformed_sequence[] = {
        0x00u, 0x00u, 0x00u, 0x03u,
        0x00u, 0x00u, 0x00u, 0x02u,
        0x00u, 0x00u, 0x00u, 0x01u, 'A',
        0x00u, 0x00u, 0x00u, 0x03u, 'N', 'O',
    };
    WalletEngineExampleError input = {
        .tag = WALLET_ENGINE_EXAMPLE_ERROR_FAILED,
        .payload = {
            .failed = {
                .kind = WALLET_ENGINE_NETWORK_TESTNET,
                .diagnostic = {diagnostic, sizeof(diagnostic)},
                .retry_after_ms = {true, UINT64_C(0x0102030405060708)},
            },
        },
    };
    WalletEngineExampleError actual = {0};
    WalletEnginePrivateRustBuffer buffer = {0};
    WalletEnginePrivateArena arena = {0};

    assert(wallet_engine_private_lower_example_error(input, &buffer)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == sizeof(expected));
    assert_bytes(buffer.data, expected, sizeof(expected));
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_OK);
    assert(actual.tag == WALLET_ENGINE_EXAMPLE_ERROR_FAILED);
    assert(actual.payload.failed.kind == WALLET_ENGINE_NETWORK_TESTNET);
    assert(actual.payload.failed.diagnostic.len == sizeof(diagnostic));
    assert_bytes(
        (const uint8_t *)actual.payload.failed.diagnostic.data,
        (const uint8_t *)diagnostic,
        sizeof(diagnostic)
    );
    assert(actual.payload.failed.retry_after_ms.has_value);
    assert(actual.payload.failed.retry_after_ms.value
        == UINT64_C(0x0102030405060708));
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_example_error(
        (WalletEngineExampleError){.tag = WALLET_ENGINE_EXAMPLE_ERROR_CANCELLED},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_OK);
    assert(buffer.len == 4u);
    assert_bytes(buffer.data, (const uint8_t[4]){0u, 0u, 0u, 1u}, 4u);
    wallet_engine_private_rustbuffer_free(buffer);

    buffer = (WalletEnginePrivateRustBuffer){0};
    assert(wallet_engine_private_lower_example_error(
        (WalletEngineExampleError){.tag = (WalletEngineExampleErrorTag)3u},
        &buffer
    ) == WALLET_ENGINE_ABI_STATUS_INVALID_ARGUMENT);
    assert(buffer.data == NULL && buffer.len == 0u && buffer.capacity == 0u);

    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(unknown_tag), sizeof(unknown_tag), unknown_tag,
    };
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(invalid_network), sizeof(invalid_network), invalid_network,
    };
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(actual.payload.failed.diagnostic.data == NULL);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(truncated_diagnostic), sizeof(truncated_diagnostic), truncated_diagnostic,
    };
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){sizeof(trailing), sizeof(trailing), trailing};
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    buffer = (WalletEnginePrivateRustBuffer){
        sizeof(malformed_sequence), sizeof(malformed_sequence), malformed_sequence,
    };
    assert(wallet_engine_private_lift_example_error(&buffer, &arena, &actual)
        == WALLET_ENGINE_ABI_STATUS_PANIC);
    assert(arena.head == NULL);
    assert(live_allocations == 0u);
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
    test_optional_u64_round_trip();
    test_optional_string_round_trip();
    test_optional_flat_enum_round_trip();
    test_sequence_u64_round_trip();
    test_sequence_string_round_trip();
    test_sequence_flat_enum_round_trip();
    test_record_round_trip();
    test_nested_record_round_trip();
    test_nested_compound_round_trip();
    test_empty_record_round_trip();
    test_rich_error_round_trip();
    test_string_round_trip();
    test_empty_string_round_trip();
    test_bytes_round_trip();
    test_bounds_failures();
    test_malformed_strings();
    test_malformed_compound_values();
    assert(live_allocations == 0u);
    return EXIT_SUCCESS;
}
