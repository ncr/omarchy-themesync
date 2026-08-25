// Simulated watch receiver, built from the firmware's own decoder (../common/theme_proto.c).
//
//   omawatch encode --raw --file tokyo-night.toml | ./theme_sim
//   ./theme_sim 5448010e...        (hex on the command line)
//   ./theme_sim --selftest         (encode/decode round trip + corruption checks)
//
// Prints exactly what the ThemeManager on the watch would end up with, plus the Status
// bytes it would report back. Exit code 0 = accepted, 1 = rejected, 2 = usage.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <ctype.h>
#include "theme_proto.h"

// The firmware's built-in palette (theme.c THEME_BUILTIN): what a watch shows before any sync.
static const theme_rgb_t BUILTIN[THEME_ROLE_COUNT] = {
    {0x0a, 0x0b, 0x10}, {0x16, 0x19, 0x22}, {0x2a, 0x2f, 0x3a}, {0xe6, 0xe8, 0xee},
    {0x8b, 0x90, 0xa0}, {0x5a, 0x5f, 0x6e}, {0x00, 0xe6, 0x76}, {0x00, 0x11, 0x0a},
    {0x1f, 0x3a, 0x2c}, {0x2a, 0x2f, 0x3a}, {0xff, 0x52, 0x52}, {0xff, 0xab, 0x00},
    {0x00, 0xe6, 0x76}, {0x40, 0xc4, 0xff},
};

// The simulated ThemeManager state ("NVS").
static struct {
    theme_rgb_t colors[THEME_ROLE_COUNT];
    bool light;
    char name[THEME_PROTO_MAX_NAME + 1];
    uint16_t crc;
    uint8_t n_applied;
    theme_result_t last;
} tm;

static void tm_init(void)
{
    memcpy(tm.colors, BUILTIN, sizeof BUILTIN);
    tm.light = false;
    strcpy(tm.name, "builtin");
    tm.crc = 0;
    tm.n_applied = 0;
    tm.last = THEME_ERR_NO_THEME;
}

// Same rule as theme_apply() in the firmware: known slots present on the wire are copied,
// everything else keeps its previous value.
static theme_result_t tm_receive(const uint8_t *buf, size_t len)
{
    theme_packet_t p;
    theme_result_t r = theme_proto_decode(buf, len, &p);
    tm.last = r;
    if (r != THEME_OK) return r;
    uint8_t n = p.n_colors < THEME_ROLE_COUNT ? p.n_colors : THEME_ROLE_COUNT;
    for (uint8_t i = 0; i < n; i++) tm.colors[i] = p.colors[i];
    tm.light = p.light;
    strcpy(tm.name, p.name);
    tm.crc = p.crc;
    tm.n_applied = n;
    return THEME_OK;
}

static void print_state(bool ansi)
{
    printf("theme: %s   mode: %s   applied %u/%u slots   crc 0x%04x\n",
           tm.name[0] ? tm.name : "(unnamed)", tm.light ? "light" : "dark", tm.n_applied, THEME_ROLE_COUNT, tm.crc);
    const theme_rgb_t bg = tm.colors[THEME_BACKGROUND], sf = tm.colors[THEME_SURFACE];
    for (int i = 0; i < THEME_ROLE_COUNT; i++) {
        const theme_rgb_t c = tm.colors[i];
        if (ansi)
            printf("  %-15s #%02x%02x%02x  \x1b[48;2;%u;%u;%um      \x1b[0m  \x1b[48;2;%u;%u;%um\x1b[38;2;%u;%u;%um Aa \x1b[0m\x1b[48;2;%u;%u;%um\x1b[38;2;%u;%u;%um Aa \x1b[0m\n",
                   theme_role_name(i), c.r, c.g, c.b, c.r, c.g, c.b,
                   bg.r, bg.g, bg.b, c.r, c.g, c.b, sf.r, sf.g, sf.b, c.r, c.g, c.b);
        else
            printf("  %-15s #%02x%02x%02x\n", theme_role_name(i), c.r, c.g, c.b);
    }
    uint8_t st[THEME_STATUS_LEN];
    theme_proto_status_encode(st, tm.last, tm.crc, tm.n_applied, tm.light);
    printf("status characteristic: %02x %02x %02x %02x %02x %02x  (%s)\n", st[0], st[1], st[2], st[3], st[4], st[5], theme_result_name(tm.last));
}

static int selftest(void)
{
    theme_packet_t p;
    memset(&p, 0, sizeof p);
    p.light = true;
    strcpy(p.name, "selftest");
    for (int i = 0; i < THEME_ROLE_COUNT; i++) p.colors[i] = (theme_rgb_t){ (uint8_t)i, (uint8_t)(0x80 + i), (uint8_t)(0xff - i) };
    uint8_t buf[THEME_PROTO_MAX_PACKET];
    size_t n = theme_proto_encode(&p, buf, sizeof buf);
    int fails = 0;
#define CHECK(cond, msg) do { if (!(cond)) { printf("FAIL: %s\n", msg); fails++; } else printf("ok:   %s\n", msg); } while (0)
    CHECK(theme_crc16((const uint8_t *)"123456789", 9) == 0x29B1, "crc16 check value 0x29B1");
    CHECK(n == 5 + 42 + 2 + 8 + 2, "encoded length 59 with 8-byte name");
    theme_packet_t q;
    CHECK(theme_proto_decode(buf, n, &q) == THEME_OK, "round trip decodes");
    CHECK(q.light && strcmp(q.name, "selftest") == 0 && q.n_colors == THEME_ROLE_COUNT && memcmp(q.colors, p.colors, sizeof p.colors) == 0, "round trip payload matches");
    buf[10] ^= 1;
    CHECK(theme_proto_decode(buf, n, &q) == THEME_ERR_BAD_CRC, "bit flip -> crc mismatch");
    buf[10] ^= 1;
    CHECK(theme_proto_decode(buf, n - 1, &q) == THEME_ERR_BAD_CRC, "truncated by one -> crc mismatch");
    CHECK(theme_proto_decode(buf, 4, &q) == THEME_ERR_TRUNCATED, "4 bytes -> truncated");
    uint8_t bad[THEME_PROTO_MAX_PACKET]; memcpy(bad, buf, n); bad[0] = 0;
    CHECK(theme_proto_decode(bad, n, &q) == THEME_ERR_BAD_MAGIC, "bad magic");
    memcpy(bad, buf, n); bad[2] = 2; { uint16_t c = theme_crc16(bad, n - 2); bad[n - 2] = c & 0xff; bad[n - 1] = c >> 8; }
    CHECK(theme_proto_decode(bad, n, &q) == THEME_ERR_BAD_VERSION, "version 2 -> unsupported");
    // future sender: 16 slots + unknown tlv
    uint8_t fut[THEME_PROTO_MAX_PACKET]; size_t o = 0;
    fut[o++] = 0x54; fut[o++] = 0x48; fut[o++] = 1; fut[o++] = 0; fut[o++] = 16;
    for (int i = 0; i < 16; i++) { fut[o++] = (uint8_t)i; fut[o++] = 1; fut[o++] = 2; }
    fut[o++] = 0x7e; fut[o++] = 2; fut[o++] = 0xaa; fut[o++] = 0xbb;
    fut[o++] = 1; fut[o++] = 3; fut[o++] = 'n'; fut[o++] = 'e'; fut[o++] = 'w';
    { uint16_t c = theme_crc16(fut, o); fut[o++] = c & 0xff; fut[o++] = c >> 8; }
    CHECK(theme_proto_decode(fut, o, &q) == THEME_OK && q.n_colors == 16 && strcmp(q.name, "new") == 0 && q.colors[13].r == 13, "16-slot packet with unknown tlv accepted, 14 used");
    // old sender: 12 slots -> the rest keeps defaults
    tm_init();
    uint8_t old[THEME_PROTO_MAX_PACKET]; o = 0;
    old[o++] = 0x54; old[o++] = 0x48; old[o++] = 1; old[o++] = 0; old[o++] = 12;
    for (int i = 0; i < 12; i++) { old[o++] = 9; old[o++] = 9; old[o++] = 9; }
    { uint16_t c = theme_crc16(old, o); old[o++] = c & 0xff; old[o++] = c >> 8; }
    CHECK(tm_receive(old, o) == THEME_OK && tm.n_applied == 12 && tm.colors[THEME_INFO].r == BUILTIN[THEME_INFO].r && tm.colors[0].r == 9, "12-slot packet keeps builtin success/info");
    printf("%s\n", fails ? "SELFTEST FAILED" : "selftest passed");
    return fails ? 1 : 0;
}

static size_t read_hex(const char *s, uint8_t *out, size_t cap)
{
    size_t n = 0; int hi = -1;
    for (; *s; s++) {
        if (!isxdigit((unsigned char)*s)) continue;
        int v = isdigit((unsigned char)*s) ? *s - '0' : (tolower((unsigned char)*s) - 'a' + 10);
        if (hi < 0) hi = v; else { if (n < cap) out[n++] = (uint8_t)((hi << 4) | v); hi = -1; }
    }
    return n;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "--selftest") == 0) return selftest();
    uint8_t buf[THEME_PROTO_MAX_PACKET + 16];
    size_t len = 0;
    if (argc > 1) {
        len = read_hex(argv[1], buf, sizeof buf);
    } else {
        // raw bytes or hex text on stdin
        uint8_t raw[4096]; size_t rn = fread(raw, 1, sizeof raw, stdin);
        if (rn >= 2 && raw[0] == THEME_PROTO_MAGIC0 && raw[1] == THEME_PROTO_MAGIC1) { len = rn < sizeof buf ? rn : sizeof buf; memcpy(buf, raw, len); }
        else { raw[rn < sizeof raw ? rn : sizeof raw - 1] = 0; len = read_hex((char *)raw, buf, sizeof buf); }
    }
    if (len == 0) { fprintf(stderr, "usage: theme_sim [<hex>|--selftest] (or raw packet on stdin)\n"); return 2; }
    tm_init();
    printf("watch: received %zu bytes\n", len);
    theme_result_t r = tm_receive(buf, len);
    printf("watch: %s\n", theme_result_name(r));
    print_state(isatty(1) && !getenv("NO_COLOR"));
    return r == THEME_OK ? 0 : 1;
}
