#include "theme.h"
#include <string.h>
#include "esp_log.h"
#include "nvs.h"

static const char *TAG = "theme";

#define NVS_NAMESPACE "theme"
#define NVS_KEY_PKT   "pkt"

theme_t theme;

// The palette a freshly flashed watch shows until it hears from the desktop. Mirrors
// `builtin_watch_theme()` on the host and BUILTIN[] in the simulator.
static const theme_rgb_t THEME_BUILTIN[THEME_ROLE_COUNT] = {
    [THEME_BACKGROUND]     = {0x0a, 0x0b, 0x10},
    [THEME_SURFACE]        = {0x16, 0x19, 0x22},
    [THEME_SURFACE_ALT]    = {0x2a, 0x2f, 0x3a},
    [THEME_TEXT_PRIMARY]   = {0xe6, 0xe8, 0xee},
    [THEME_TEXT_SECONDARY] = {0x8b, 0x90, 0xa0},
    [THEME_TEXT_DISABLED]  = {0x5a, 0x5f, 0x6e},
    [THEME_ACCENT]         = {0x00, 0xe6, 0x76},
    [THEME_ON_ACCENT]      = {0x00, 0x11, 0x0a},
    [THEME_SELECTION]      = {0x1f, 0x3a, 0x2c},
    [THEME_DIVIDER]        = {0x2a, 0x2f, 0x3a},
    [THEME_DANGER]         = {0xff, 0x52, 0x52},
    [THEME_WARNING]        = {0xff, 0xab, 0x00},
    [THEME_SUCCESS]        = {0x00, 0xe6, 0x76},
    [THEME_INFO]           = {0x40, 0xc4, 0xff},
};

static lv_style_t       s_styles[THEME_STYLE_COUNT];
static bool             s_styles_ready = false;
static uint8_t          s_last_packet[THEME_PROTO_MAX_PACKET];
static size_t           s_last_packet_len = 0;
static theme_result_t   s_last_result = THEME_ERR_NO_THEME;
static theme_listener_t s_listener = NULL;

static inline lv_color_t rgb(theme_rgb_t c) { return lv_color_make(c.r, c.g, c.b); }

// Rec.601 luma, 0..255. Cheap enough to run on every apply.
static uint32_t luma(lv_color_t c)
{
    return (299u * c.red + 587u * c.green + 114u * c.blue) / 1000u;
}

// Text colour to put on top of an arbitrary status colour: whichever of background /
// text_primary is further away in luma. Derived on the watch because the desktop only
// sends `on_accent`; adding on_warning etc. to the wire would be slot bloat for one label.
static lv_color_t on_color(lv_color_t base)
{
    lv_color_t a = THEME_COLOR(THEME_BACKGROUND), b = THEME_COLOR(THEME_TEXT_PRIMARY);
    uint32_t lb = luma(base), la = luma(a), lbb = luma(b);
    uint32_t da = la > lb ? la - lb : lb - la;
    uint32_t db = lbb > lb ? lbb - lb : lb - lbb;
    return da >= db ? a : b;
}

static void styles_init_once(void)
{
    if (s_styles_ready) return;
    for (int i = 0; i < THEME_STYLE_COUNT; i++) lv_style_init(&s_styles[i]);

    lv_style_set_bg_opa(&s_styles[THEME_STYLE_SCREEN], LV_OPA_COVER);

    lv_style_set_bg_opa(&s_styles[THEME_STYLE_CARD], LV_OPA_COVER);
    lv_style_set_border_width(&s_styles[THEME_STYLE_CARD], 0);

    lv_style_set_bg_opa(&s_styles[THEME_STYLE_WELL], LV_OPA_COVER);
    lv_style_set_bg_opa(&s_styles[THEME_STYLE_SELECTION], LV_OPA_COVER);
    lv_style_set_bg_opa(&s_styles[THEME_STYLE_DIVIDER], LV_OPA_COVER);
    lv_style_set_border_width(&s_styles[THEME_STYLE_DIVIDER], 1);

    for (int i = THEME_STYLE_BUTTON_ACCENT; i <= THEME_STYLE_INDICATOR_INFO; i++)
        lv_style_set_bg_opa(&s_styles[i], LV_OPA_COVER);
    lv_style_set_shadow_width(&s_styles[THEME_STYLE_BUTTON_ACCENT], 0);   // sw renderer: shadows cost a frame
    lv_style_set_shadow_width(&s_styles[THEME_STYLE_BUTTON_WARNING], 0);

    lv_style_set_bg_opa(&s_styles[THEME_STYLE_FRAME], LV_OPA_TRANSP);
    lv_style_set_border_opa(&s_styles[THEME_STYLE_FRAME], LV_OPA_COVER);
    lv_style_set_border_width(&s_styles[THEME_STYLE_FRAME], 1);

    s_styles_ready = true;
}

// Push the palette into the shared styles and let LVGL repaint everything that uses them.
static void styles_refresh(void)
{
    styles_init_once();
    lv_style_set_bg_color(&s_styles[THEME_STYLE_SCREEN], THEME_COLOR(THEME_BACKGROUND));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_CARD], THEME_COLOR(THEME_SURFACE));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_WELL], THEME_COLOR(THEME_SURFACE_ALT));

    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT], THEME_COLOR(THEME_TEXT_PRIMARY));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_SECONDARY], THEME_COLOR(THEME_TEXT_SECONDARY));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_DISABLED], THEME_COLOR(THEME_TEXT_DISABLED));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_ACCENT], THEME_COLOR(THEME_ACCENT));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_ON_ACCENT], THEME_COLOR(THEME_ON_ACCENT));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_ON_WARNING], on_color(THEME_COLOR(THEME_WARNING)));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_DANGER], THEME_COLOR(THEME_DANGER));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_WARNING], THEME_COLOR(THEME_WARNING));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_SUCCESS], THEME_COLOR(THEME_SUCCESS));
    lv_style_set_text_color(&s_styles[THEME_STYLE_TEXT_INFO], THEME_COLOR(THEME_INFO));

    lv_style_set_bg_color(&s_styles[THEME_STYLE_BUTTON_ACCENT], THEME_COLOR(THEME_ACCENT));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_BUTTON_WARNING], THEME_COLOR(THEME_WARNING));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_INDICATOR_ACCENT], THEME_COLOR(THEME_ACCENT));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_INDICATOR_WARNING], THEME_COLOR(THEME_WARNING));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_INDICATOR_DANGER], THEME_COLOR(THEME_DANGER));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_INDICATOR_SUCCESS], THEME_COLOR(THEME_SUCCESS));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_INDICATOR_INFO], THEME_COLOR(THEME_INFO));

    lv_style_set_bg_color(&s_styles[THEME_STYLE_SELECTION], THEME_COLOR(THEME_SELECTION));
    lv_style_set_bg_color(&s_styles[THEME_STYLE_DIVIDER], THEME_COLOR(THEME_DIVIDER));
    lv_style_set_border_color(&s_styles[THEME_STYLE_DIVIDER], THEME_COLOR(THEME_DIVIDER));
    lv_style_set_border_color(&s_styles[THEME_STYLE_FRAME], THEME_COLOR(THEME_ACCENT));

    // NULL = "some style changed": every object re-resolves its styles and invalidates.
    // One full repaint per theme change is the whole cost.
    lv_obj_report_style_change(NULL);
}

lv_style_t *theme_style(theme_style_id_t id)
{
    styles_init_once();
    return &s_styles[id < THEME_STYLE_COUNT ? id : THEME_STYLE_TEXT];
}

static void set_builtin(void)
{
    for (int i = 0; i < THEME_ROLE_COUNT; i++) theme.color[i] = rgb(THEME_BUILTIN[i]);
    theme.light = false;
    strcpy(theme.name, "builtin");
    theme.crc = 0;
    theme.n_applied = 0;
}

void theme_apply(const theme_packet_t *pkt)
{
    const uint8_t n = pkt->n_colors < THEME_ROLE_COUNT ? pkt->n_colors : THEME_ROLE_COUNT;
    for (uint8_t i = 0; i < n; i++) theme.color[i] = rgb(pkt->colors[i]);
    // slots the sender did not know keep whatever they had (built-in or previous theme)
    theme.light = pkt->light;
    strncpy(theme.name, pkt->name, THEME_PROTO_MAX_NAME);
    theme.name[THEME_PROTO_MAX_NAME] = '\0';
    theme.crc = pkt->crc;
    theme.n_applied = n;
    styles_refresh();
    ESP_LOGI(TAG, "applied '%s' (%s, %u colours, crc 0x%04x)", theme.name, theme.light ? "light" : "dark", n, theme.crc);
    if (s_listener) s_listener(&theme);
}

static bool nvs_save(const uint8_t *buf, size_t len)
{
    nvs_handle_t h;
    if (nvs_open(NVS_NAMESPACE, NVS_READWRITE, &h) != ESP_OK) return false;
    esp_err_t e = nvs_set_blob(h, NVS_KEY_PKT, buf, len);
    if (e == ESP_OK) e = nvs_commit(h);
    nvs_close(h);
    if (e != ESP_OK) ESP_LOGW(TAG, "nvs save failed: %s", esp_err_to_name(e));
    return e == ESP_OK;
}

static size_t nvs_load(uint8_t *buf, size_t cap)
{
    nvs_handle_t h;
    if (nvs_open(NVS_NAMESPACE, NVS_READONLY, &h) != ESP_OK) return 0;
    size_t len = cap;
    esp_err_t e = nvs_get_blob(h, NVS_KEY_PKT, buf, &len);
    nvs_close(h);
    return e == ESP_OK ? len : 0;
}

theme_result_t theme_apply_packet(const uint8_t *buf, size_t len, bool persist)
{
    theme_packet_t pkt;
    theme_result_t r = theme_proto_decode(buf, len, &pkt);
    s_last_result = r;
    if (r != THEME_OK) {
        ESP_LOGW(TAG, "rejected %u-byte packet: %s", (unsigned)len, theme_result_name(r));
        return r;
    }
    theme_apply(&pkt);
    memcpy(s_last_packet, buf, len);
    s_last_packet_len = len;
    if (persist) nvs_save(buf, len);
    return THEME_OK;
}

size_t theme_last_packet(uint8_t *buf, size_t cap)
{
    size_t n = s_last_packet_len < cap ? s_last_packet_len : cap;
    memcpy(buf, s_last_packet, n);
    return n;
}

theme_result_t theme_last_result(void) { return s_last_result; }

void theme_status_bytes(uint8_t out[THEME_STATUS_LEN])
{
    theme_proto_status_encode(out, s_last_result, theme.crc, theme.n_applied, theme.light);
}

void theme_set_listener(theme_listener_t fn) { s_listener = fn; }

void theme_init(void)
{
    set_builtin();
    styles_refresh();
    uint8_t buf[THEME_PROTO_MAX_PACKET];
    size_t len = nvs_load(buf, sizeof buf);
    if (len) {
        if (theme_apply_packet(buf, len, false) == THEME_OK)
            ESP_LOGI(TAG, "restored '%s' from NVS", theme.name);
        else
            ESP_LOGW(TAG, "stored theme invalid; using built-in palette");
    } else {
        ESP_LOGI(TAG, "no stored theme; using built-in palette");
    }
}
