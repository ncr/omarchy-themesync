// Theme Protocol v1 — portable C reference implementation (no OS, no LVGL, no malloc).
// Normative spec: protocol/THEME_PROTOCOL.md. Rust twin: host/src/protocol.rs.
//
// Used unchanged by the ESP32 firmware (watch/esp32-lvgl/theme.c) and by the host-side
// simulator (watch/sim/theme_sim.c), so what the simulator accepts, the watch accepts.
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define THEME_PROTO_VERSION        1
#define THEME_PROTO_MAGIC0         0x54   // 'T'
#define THEME_PROTO_MAGIC1         0x48   // 'H'  -> ThemeState packet
#define THEME_PROTO_CTRL_MAGIC1    0x43   // 'C'  -> Control packet ("TC")
#define THEME_PROTO_FLAG_LIGHT     0x01
#define THEME_PROTO_TLV_NAME       0x01
#define THEME_PROTO_HEADER_LEN     5
#define THEME_PROTO_CRC_LEN        2
#define THEME_PROTO_MAX_PACKET     240    // hard cap on one ThemeState write
#define THEME_PROTO_MAX_NAME       32     // bytes of UTF-8, not counting the NUL

// Semantic colour roles = wire slots. Append-only; never renumber.
enum theme_role {
    THEME_BACKGROUND = 0,
    THEME_SURFACE,
    THEME_SURFACE_ALT,
    THEME_TEXT_PRIMARY,
    THEME_TEXT_SECONDARY,
    THEME_TEXT_DISABLED,
    THEME_ACCENT,
    THEME_ON_ACCENT,
    THEME_SELECTION,
    THEME_DIVIDER,
    THEME_DANGER,
    THEME_WARNING,
    THEME_SUCCESS,
    THEME_INFO,
    THEME_ROLE_COUNT
};

typedef struct { uint8_t r, g, b; } theme_rgb_t;

// A decoded ThemeState. `n_colors` is what the sender put on the wire (it may be more or
// fewer than THEME_ROLE_COUNT); `colors[i]` is valid for i < n_colors && i < THEME_ROLE_COUNT.
typedef struct {
    uint8_t     version;
    bool        light;
    uint8_t     n_colors;
    theme_rgb_t colors[THEME_ROLE_COUNT];
    char        name[THEME_PROTO_MAX_NAME + 1];   // "" when the packet carried no name
    uint16_t    crc;                               // the packet's own crc16 (ack token)
} theme_packet_t;

// Result codes, also the Status characteristic's `result` byte.
typedef enum {
    THEME_OK              = 0,
    THEME_ERR_BAD_MAGIC   = 1,
    THEME_ERR_BAD_VERSION = 2,
    THEME_ERR_BAD_CRC     = 3,
    THEME_ERR_TRUNCATED   = 4,
    THEME_ERR_BAD_TLV     = 5,
    THEME_ERR_NO_THEME    = 6,   // status only: nothing applied since boot
} theme_result_t;

// CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF). theme_crc16("123456789", 9) == 0x29B1.
uint16_t theme_crc16(const uint8_t *data, size_t len);

// Validate + parse. `out` is only written on THEME_OK. Order of checks matches the Rust
// decoder: length, magic, version, crc, colour block, TLVs.
theme_result_t theme_proto_decode(const uint8_t *buf, size_t len, theme_packet_t *out);

// Serialize `in` (n_colors is forced to THEME_ROLE_COUNT). Returns bytes written, 0 if
// `cap` is too small. Used by the simulator's self-test and by tests.
size_t theme_proto_encode(const theme_packet_t *in, uint8_t *buf, size_t cap);

// Status characteristic (6 bytes): [ver][result][crc lo][crc hi][n_applied][flags]
#define THEME_STATUS_LEN 6
void theme_proto_status_encode(uint8_t out[THEME_STATUS_LEN], theme_result_t result,
                               uint16_t applied_crc, uint8_t n_applied, bool light);

// Info characteristic (4 bytes): [proto_min][proto_max][max_colors][features]
#define THEME_INFO_LEN         4
#define THEME_FEATURE_CONTROL  0x01   // the watch can send Control notifications
#define THEME_FEATURE_PERSIST  0x02   // the watch stores the last theme across reboots
void theme_proto_info_encode(uint8_t out[THEME_INFO_LEN], uint8_t features);

// Control packet (4 bytes, watch -> desktop): "TC" ver op
enum theme_control {
    THEME_CTRL_NEXT        = 1,
    THEME_CTRL_PREV        = 2,
    THEME_CTRL_TOGGLE_MODE = 3,
    THEME_CTRL_RESEND      = 4,
};
#define THEME_CONTROL_LEN 4
void theme_proto_control_encode(uint8_t out[THEME_CONTROL_LEN], uint8_t op);

const char *theme_role_name(int role);
const char *theme_result_name(theme_result_t r);

#ifdef __cplusplus
}
#endif
