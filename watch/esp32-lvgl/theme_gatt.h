// Theme Protocol v1 GATT service on NimBLE (ESP-IDF).
//
//   service        7e450001-5029-4337-8dde-aaefb009b2df
//   theme state    7e450002-…  write (long writes ok) / read   ThemeState packet
//   status         7e450003-…  read / notify                   6 bytes, sent after every write
//   control        7e450004-…  notify / read                   4 bytes, watch -> desktop
//   info           7e450005-…  read                            4 bytes, version negotiation
//
// Writes are applied with the LVGL lock held (lvgl_port_lock) and persisted to NVS, then
// the Status characteristic is updated and notified. The desktop confirms by matching
// Status.crc against the packet it sent.
#pragma once
#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Register the service and set the GAP device name. Must run after nimble_port_init() and
// BEFORE the host task starts (nimble_port_freertos_init): services cannot be added later.
int theme_gatt_register(const char *device_name);

// Start connectable advertising with the service UUID (name in the scan response).
// Call once the host has synced; re-called automatically after a disconnect.
int theme_gatt_advertise(uint8_t own_addr_type);

bool theme_gatt_connected(void);

// Notify the desktop with a THEME_CTRL_* opcode. Returns 0, or BLE_HS_ENOTCONN when
// nobody is connected / subscribed.
int theme_gatt_send_control(uint8_t op);

// Link state changes (called from the NimBLE host task; do not touch LVGL without locking).
typedef void (*theme_gatt_state_cb_t)(bool connected);
void theme_gatt_set_state_cb(theme_gatt_state_cb_t cb);

#ifdef __cplusplus
}
#endif
