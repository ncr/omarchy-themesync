#include "theme_proto.h"
#include <string.h>

uint16_t theme_crc16(const uint8_t *data, size_t len)
{
    uint16_t crc = 0xFFFF;
    for (size_t i = 0; i < len; i++) {
        crc ^= (uint16_t)data[i] << 8;
        for (int b = 0; b < 8; b++)
            crc = (crc & 0x8000) ? (uint16_t)((crc << 1) ^ 0x1021) : (uint16_t)(crc << 1);
    }
    return crc;
}

theme_result_t theme_proto_decode(const uint8_t *buf, size_t len, theme_packet_t *out)
{
    if (len > THEME_PROTO_MAX_PACKET) return THEME_ERR_TRUNCATED;   // "too long" shares the length code
    if (len < THEME_PROTO_HEADER_LEN + THEME_PROTO_CRC_LEN) return THEME_ERR_TRUNCATED;
    if (buf[0] != THEME_PROTO_MAGIC0 || buf[1] != THEME_PROTO_MAGIC1) return THEME_ERR_BAD_MAGIC;
    if (buf[2] != THEME_PROTO_VERSION) return THEME_ERR_BAD_VERSION;

    const size_t body_end = len - THEME_PROTO_CRC_LEN;
    const uint16_t expected = (uint16_t)buf[body_end] | ((uint16_t)buf[body_end + 1] << 8);
    if (theme_crc16(buf, body_end) != expected) return THEME_ERR_BAD_CRC;

    theme_packet_t p;
    memset(&p, 0, sizeof p);
    p.version  = buf[2];
    p.light    = (buf[3] & THEME_PROTO_FLAG_LIGHT) != 0;
    p.n_colors = buf[4];
    p.crc      = expected;

    const size_t colors_end = THEME_PROTO_HEADER_LEN + 3u * p.n_colors;
    if (colors_end > body_end) return THEME_ERR_TRUNCATED;
    const uint8_t n = p.n_colors < THEME_ROLE_COUNT ? p.n_colors : THEME_ROLE_COUNT;
    for (uint8_t i = 0; i < n; i++) {
        const uint8_t *c = buf + THEME_PROTO_HEADER_LEN + 3u * i;
        p.colors[i].r = c[0];
        p.colors[i].g = c[1];
        p.colors[i].b = c[2];
    }

    size_t i = colors_end;
    while (i < body_end) {
        if (i + 2 > body_end) return THEME_ERR_BAD_TLV;
        const uint8_t tag = buf[i];
        const size_t  tl  = buf[i + 1];
        const size_t  start = i + 2, end = start + tl;
        if (end > body_end) return THEME_ERR_BAD_TLV;
        if (tag == THEME_PROTO_TLV_NAME) {
            size_t nlen = tl < THEME_PROTO_MAX_NAME ? tl : THEME_PROTO_MAX_NAME;
            memcpy(p.name, buf + start, nlen);
            p.name[nlen] = '\0';
        }
        // unknown tags: skipped, by design
        i = end;
    }

    *out = p;
    return THEME_OK;
}

size_t theme_proto_encode(const theme_packet_t *in, uint8_t *buf, size_t cap)
{
    const size_t name_len = strnlen(in->name, THEME_PROTO_MAX_NAME);
    const size_t need = THEME_PROTO_HEADER_LEN + 3u * THEME_ROLE_COUNT + (name_len ? 2 + name_len : 0) + THEME_PROTO_CRC_LEN;
    if (cap < need) return 0;
    size_t o = 0;
    buf[o++] = THEME_PROTO_MAGIC0;
    buf[o++] = THEME_PROTO_MAGIC1;
    buf[o++] = THEME_PROTO_VERSION;
    buf[o++] = in->light ? THEME_PROTO_FLAG_LIGHT : 0;
    buf[o++] = THEME_ROLE_COUNT;
    for (int i = 0; i < THEME_ROLE_COUNT; i++) {
        buf[o++] = in->colors[i].r;
        buf[o++] = in->colors[i].g;
        buf[o++] = in->colors[i].b;
    }
    if (name_len) {
        buf[o++] = THEME_PROTO_TLV_NAME;
        buf[o++] = (uint8_t)name_len;
        memcpy(buf + o, in->name, name_len);
        o += name_len;
    }
    const uint16_t crc = theme_crc16(buf, o);
    buf[o++] = (uint8_t)(crc & 0xFF);
    buf[o++] = (uint8_t)(crc >> 8);
    return o;
}

void theme_proto_status_encode(uint8_t out[THEME_STATUS_LEN], theme_result_t result,
                               uint16_t applied_crc, uint8_t n_applied, bool light)
{
    out[0] = THEME_PROTO_VERSION;
    out[1] = (uint8_t)result;
    out[2] = (uint8_t)(applied_crc & 0xFF);
    out[3] = (uint8_t)(applied_crc >> 8);
    out[4] = n_applied;
    out[5] = light ? THEME_PROTO_FLAG_LIGHT : 0;
}

void theme_proto_info_encode(uint8_t out[THEME_INFO_LEN], uint8_t features)
{
    out[0] = THEME_PROTO_VERSION;   // proto_min
    out[1] = THEME_PROTO_VERSION;   // proto_max
    out[2] = THEME_ROLE_COUNT;
    out[3] = features;
}

void theme_proto_control_encode(uint8_t out[THEME_CONTROL_LEN], uint8_t op)
{
    out[0] = THEME_PROTO_MAGIC0;
    out[1] = THEME_PROTO_CTRL_MAGIC1;
    out[2] = THEME_PROTO_VERSION;
    out[3] = op;
}

const char *theme_role_name(int role)
{
    static const char *const names[THEME_ROLE_COUNT] = {
        "background", "surface", "surface_alt", "text_primary", "text_secondary",
        "text_disabled", "accent", "on_accent", "selection", "divider",
        "danger", "warning", "success", "info",
    };
    return (role >= 0 && role < THEME_ROLE_COUNT) ? names[role] : "?";
}

const char *theme_result_name(theme_result_t r)
{
    switch (r) {
    case THEME_OK:              return "ok";
    case THEME_ERR_BAD_MAGIC:   return "bad magic";
    case THEME_ERR_BAD_VERSION: return "unsupported version";
    case THEME_ERR_BAD_CRC:     return "crc mismatch";
    case THEME_ERR_TRUNCATED:   return "truncated";
    case THEME_ERR_BAD_TLV:     return "malformed tlv";
    case THEME_ERR_NO_THEME:    return "no theme applied";
    default:                    return "?";
    }
}
