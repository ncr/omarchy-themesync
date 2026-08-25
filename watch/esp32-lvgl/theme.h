// ThemeManager for an LVGL 9 UI on ESP-IDF: the active semantic palette, shared styles
// that repaint every widget on a theme change, and NVS persistence of the last theme.
//
// Rule for UI code: never call lv_obj_set_style_*_color() with a constant. Add one of the
// shared styles instead:
//     lv_obj_add_style(card,  theme_style(THEME_STYLE_CARD), 0);
//     lv_obj_add_style(label, theme_style(THEME_STYLE_TEXT_SECONDARY), 0);
//     lv_obj_add_style(bar,   theme_style(THEME_STYLE_WELL), 0);
//     lv_obj_add_style(bar,   theme_style(THEME_STYLE_INDICATOR_ACCENT), LV_PART_INDICATOR);
// theme_apply() rewrites the styles' colours and lv_obj_report_style_change() repaints.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "lvgl.h"
#include "theme_proto.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    lv_color_t color[THEME_ROLE_COUNT];   // indexed by enum theme_role
    bool       light;
    char       name[THEME_PROTO_MAX_NAME + 1];
    uint16_t   crc;          // of the packet that produced it; 0 for the built-in palette
    uint8_t    n_applied;    // colour slots taken from that packet; 0 = built-in
} theme_t;

// The active palette. For the rare one-off draw that needs a raw lv_color_t.
extern theme_t theme;
#define THEME_COLOR(role) (theme.color[(role)])

typedef enum {
    THEME_STYLE_SCREEN,            // bg = background
    THEME_STYLE_CARD,              // bg = surface, no border
    THEME_STYLE_WELL,              // bg = surface_alt (bar tracks, insets, neutral buttons)
    THEME_STYLE_TEXT,              // text = text_primary
    THEME_STYLE_TEXT_SECONDARY,    // text = text_secondary
    THEME_STYLE_TEXT_DISABLED,     // text = text_disabled
    THEME_STYLE_TEXT_ACCENT,       // text = accent
    THEME_STYLE_TEXT_ON_ACCENT,    // text = on_accent   (labels inside accent buttons)
    THEME_STYLE_TEXT_ON_WARNING,   // text = derived on-colour for warning buttons
    THEME_STYLE_TEXT_DANGER,
    THEME_STYLE_TEXT_WARNING,
    THEME_STYLE_TEXT_SUCCESS,
    THEME_STYLE_TEXT_INFO,
    THEME_STYLE_BUTTON_ACCENT,     // bg = accent
    THEME_STYLE_BUTTON_WARNING,    // bg = warning
    THEME_STYLE_INDICATOR_ACCENT,  // bg = accent    (LV_PART_INDICATOR of bars/sliders)
    THEME_STYLE_INDICATOR_WARNING,
    THEME_STYLE_INDICATOR_DANGER,
    THEME_STYLE_INDICATOR_SUCCESS,
    THEME_STYLE_INDICATOR_INFO,
    THEME_STYLE_SELECTION,         // bg = selection
    THEME_STYLE_DIVIDER,           // bg + border = divider
    THEME_STYLE_FRAME,             // 1px border = accent, transparent bg (screen-edge ring)
    THEME_STYLE_COUNT
} theme_style_id_t;

lv_style_t *theme_style(theme_style_id_t id);

// Built-in palette + styles, then the last persisted theme (NVS namespace "theme", key
// "pkt") if there is one. Call after nvs_flash_init() and lv_init(), before the UI is built
// (or with the LVGL lock held).
void theme_init(void);

// Apply a decoded packet. LVGL lock must be held by the caller.
void theme_apply(const theme_packet_t *pkt);

// Decode + apply + optionally persist a raw ThemeState packet. LVGL lock must be held.
// Returns what the Status characteristic should report.
theme_result_t theme_apply_packet(const uint8_t *buf, size_t len, bool persist);

// The last raw packet applied (what the readable Theme State characteristic returns).
size_t theme_last_packet(uint8_t *buf, size_t cap);

theme_result_t theme_last_result(void);
void theme_status_bytes(uint8_t out[THEME_STATUS_LEN]);

// Called (LVGL lock held) after every successful apply, e.g. to refresh a "theme: X" label.
typedef void (*theme_listener_t)(const theme_t *t);
void theme_set_listener(theme_listener_t fn);

#ifdef __cplusplus
}
#endif
