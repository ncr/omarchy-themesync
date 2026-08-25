#include "theme_gatt.h"
#include <string.h>
#include "esp_log.h"
#include "host/ble_hs.h"
#include "host/ble_uuid.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#include "esp_lvgl_port.h"
#include "theme.h"
#include "theme_proto.h"

static const char *TAG = "theme_gatt";

// 7e45000X-5029-4337-8dde-aaefb009b2df, least-significant byte first as NimBLE wants it.
#define THEME_UUID128(x) BLE_UUID128_INIT( \
    0xdf, 0xb2, 0x09, 0xb0, 0xef, 0xaa, 0xde, 0x8d, \
    0x37, 0x43, 0x29, 0x50, (x), 0x00, 0x45, 0x7e)

static const ble_uuid128_t UUID_SVC     = THEME_UUID128(0x01);
static const ble_uuid128_t UUID_THEME   = THEME_UUID128(0x02);
static const ble_uuid128_t UUID_STATUS  = THEME_UUID128(0x03);
static const ble_uuid128_t UUID_CONTROL = THEME_UUID128(0x04);
static const ble_uuid128_t UUID_INFO    = THEME_UUID128(0x05);

static uint16_t h_theme, h_status, h_control, h_info;
static uint16_t s_conn = BLE_HS_CONN_HANDLE_NONE;
static uint8_t  s_own_addr_type;
static uint8_t  s_last_control[THEME_CONTROL_LEN];
static theme_gatt_state_cb_t s_state_cb;

static int gap_event(struct ble_gap_event *ev, void *arg);

static int access_cb(uint16_t conn_handle, uint16_t attr_handle, struct ble_gatt_access_ctxt *ctxt, void *arg)
{
    (void)arg;
    if (ctxt->op == BLE_GATT_ACCESS_OP_READ_CHR) {
        if (attr_handle == h_theme) {
            uint8_t buf[THEME_PROTO_MAX_PACKET];
            size_t n = theme_last_packet(buf, sizeof buf);
            return os_mbuf_append(ctxt->om, buf, n) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
        }
        if (attr_handle == h_status) {
            uint8_t st[THEME_STATUS_LEN];
            theme_status_bytes(st);
            return os_mbuf_append(ctxt->om, st, sizeof st) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
        }
        if (attr_handle == h_info) {
            uint8_t info[THEME_INFO_LEN];
            theme_proto_info_encode(info, THEME_FEATURE_CONTROL | THEME_FEATURE_PERSIST);
            return os_mbuf_append(ctxt->om, info, sizeof info) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
        }
        if (attr_handle == h_control) {
            return os_mbuf_append(ctxt->om, s_last_control, sizeof s_last_control) == 0 ? 0 : BLE_ATT_ERR_INSUFFICIENT_RES;
        }
        return BLE_ATT_ERR_UNLIKELY;
    }

    if (ctxt->op == BLE_GATT_ACCESS_OP_WRITE_CHR && attr_handle == h_theme) {
        // A long write (prepare + execute) arrives here already reassembled.
        uint8_t buf[THEME_PROTO_MAX_PACKET];
        uint16_t len = 0;
        if (OS_MBUF_PKTLEN(ctxt->om) > sizeof buf) {
            ESP_LOGW(TAG, "theme write of %u bytes rejected (max %u)", OS_MBUF_PKTLEN(ctxt->om), (unsigned)sizeof buf);
            return BLE_ATT_ERR_INVALID_ATTR_VALUE_LEN;
        }
        if (ble_hs_mbuf_to_flat(ctxt->om, buf, sizeof buf, &len) != 0) return BLE_ATT_ERR_UNLIKELY;

        // Recolour under the LVGL lock: the shared styles are read by the render task.
        theme_result_t r;
        if (lvgl_port_lock(200)) {
            r = theme_apply_packet(buf, len, true);
            lvgl_port_unlock();
        } else {
            ESP_LOGW(TAG, "LVGL busy; theme write dropped");
            return BLE_ATT_ERR_UNLIKELY;
        }
        ESP_LOGI(TAG, "theme write %u bytes from conn %d -> %s", len, conn_handle, theme_result_name(r));
        // The result (and the crc of what was applied) is the acknowledgement.
        ble_gatts_chr_updated(h_status);
        return 0;   // the Status characteristic carries the verdict, not an ATT error
    }
    return BLE_ATT_ERR_UNLIKELY;
}

static const struct ble_gatt_svc_def SERVICES[] = {
    {
        .type = BLE_GATT_SVC_TYPE_PRIMARY,
        .uuid = &UUID_SVC.u,
        .characteristics = (struct ble_gatt_chr_def[]) {
            { .uuid = &UUID_THEME.u,   .access_cb = access_cb, .val_handle = &h_theme,
              .flags = BLE_GATT_CHR_F_WRITE | BLE_GATT_CHR_F_READ },
            { .uuid = &UUID_STATUS.u,  .access_cb = access_cb, .val_handle = &h_status,
              .flags = BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_NOTIFY },
            { .uuid = &UUID_CONTROL.u, .access_cb = access_cb, .val_handle = &h_control,
              .flags = BLE_GATT_CHR_F_READ | BLE_GATT_CHR_F_NOTIFY },
            { .uuid = &UUID_INFO.u,    .access_cb = access_cb, .val_handle = &h_info,
              .flags = BLE_GATT_CHR_F_READ },
            { 0 },
        },
    },
    { 0 },
};

int theme_gatt_register(const char *device_name)
{
    ble_svc_gap_init();
    ble_svc_gatt_init();
    int rc = ble_gatts_count_cfg(SERVICES);
    if (rc == 0) rc = ble_gatts_add_svcs(SERVICES);
    if (rc == 0 && device_name) rc = ble_svc_gap_device_name_set(device_name);
    ESP_LOGI(TAG, "service registered rc=%d", rc);
    return rc;
}

int theme_gatt_advertise(uint8_t own_addr_type)
{
    s_own_addr_type = own_addr_type;

    struct ble_hs_adv_fields adv = {0};
    adv.flags = BLE_HS_ADV_F_DISC_GEN | BLE_HS_ADV_F_BREDR_UNSUP;
    adv.uuids128 = (ble_uuid128_t *)&UUID_SVC;   // 18 bytes: the whole point of the adv packet
    adv.num_uuids128 = 1;
    adv.uuids128_is_complete = 1;
    int rc = ble_gap_adv_set_fields(&adv);
    if (rc) { ESP_LOGE(TAG, "adv fields rc=%d", rc); return rc; }

    struct ble_hs_adv_fields rsp = {0};
    const char *name = ble_svc_gap_device_name();
    rsp.name = (uint8_t *)name;
    rsp.name_len = strlen(name);
    rsp.name_is_complete = 1;
    rc = ble_gap_adv_rsp_set_fields(&rsp);
    if (rc) { ESP_LOGE(TAG, "scan rsp rc=%d", rc); return rc; }

    struct ble_gap_adv_params params = {0};
    params.conn_mode = BLE_GAP_CONN_MODE_UND;
    params.disc_mode = BLE_GAP_DISC_MODE_GEN;
    params.itvl_min = BLE_GAP_ADV_ITVL_MS(100);   // quick to find, light on the radio/display
    params.itvl_max = BLE_GAP_ADV_ITVL_MS(150);
    rc = ble_gap_adv_start(own_addr_type, NULL, BLE_HS_FOREVER, &params, gap_event, NULL);
    if (rc && rc != BLE_HS_EALREADY) ESP_LOGE(TAG, "adv start rc=%d", rc);
    else ESP_LOGI(TAG, "advertising as '%s'", name);
    return rc;
}

static int gap_event(struct ble_gap_event *ev, void *arg)
{
    (void)arg;
    switch (ev->type) {
    case BLE_GAP_EVENT_CONNECT:
        if (ev->connect.status == 0) {
            s_conn = ev->connect.conn_handle;
            ESP_LOGI(TAG, "desktop connected (conn %d)", s_conn);
            if (s_state_cb) s_state_cb(true);
        } else {
            ESP_LOGW(TAG, "connect failed status=%d; advertising again", ev->connect.status);
            theme_gatt_advertise(s_own_addr_type);
        }
        return 0;
    case BLE_GAP_EVENT_DISCONNECT:
        ESP_LOGI(TAG, "desktop disconnected reason=%d; advertising again", ev->disconnect.reason);
        s_conn = BLE_HS_CONN_HANDLE_NONE;
        if (s_state_cb) s_state_cb(false);
        theme_gatt_advertise(s_own_addr_type);
        return 0;
    case BLE_GAP_EVENT_ADV_COMPLETE:
        theme_gatt_advertise(s_own_addr_type);
        return 0;
    case BLE_GAP_EVENT_MTU:
        ESP_LOGI(TAG, "mtu %d on conn %d", ev->mtu.value, ev->mtu.conn_handle);
        return 0;
    case BLE_GAP_EVENT_SUBSCRIBE:
        ESP_LOGI(TAG, "subscribe attr %d notify=%d", ev->subscribe.attr_handle, ev->subscribe.cur_notify);
        return 0;
    default:
        return 0;
    }
}

bool theme_gatt_connected(void) { return s_conn != BLE_HS_CONN_HANDLE_NONE; }

int theme_gatt_send_control(uint8_t op)
{
    theme_proto_control_encode(s_last_control, op);
    if (s_conn == BLE_HS_CONN_HANDLE_NONE) return BLE_HS_ENOTCONN;
    struct os_mbuf *om = ble_hs_mbuf_from_flat(s_last_control, sizeof s_last_control);
    if (!om) return BLE_HS_ENOMEM;
    int rc = ble_gatts_notify_custom(s_conn, h_control, om);
    ESP_LOGI(TAG, "control op %u -> rc=%d", op, rc);
    return rc;
}

void theme_gatt_set_state_cb(theme_gatt_state_cb_t cb) { s_state_cb = cb; }
