import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// The themesync daemon in the Omarchy bar.
//
// The bar shows one watch mark: lit while the beacon is on the air and a watch is paired,
// dimmed while there is nothing to show (no daemon, no key, nothing to send), red when the
// daemon needs a person (counter locked, beacon registration failing). The panel lists the
// daemon's state and offers the two things people do by hand: send the theme again and
// push the theme list to the watch.
//
// It talks to the daemon the way the CLI does — one JSON line per connection on
// $XDG_RUNTIME_DIR/themesync.sock, reply, close (host/src/transport/ipc.rs) — so it needs
// no binary on the shell's PATH and costs the daemon one in-memory answer per refresh.
Panel {
  id: root
  moduleName: "io.github.ncr.themesync"
  ipcTarget: "io.github.ncr.themesync"

  readonly property int refreshMs: Math.max(2, Number(setting("refreshIntervalSec", 10)) || 10) * 1000
  readonly property bool hideWhenNoDaemon: setting("hideWhenNoDaemon", false) === true
  readonly property string socketPath: {
    var explicit = Quickshell.env("THEMESYNC_SOCKET")
    if (explicit) return explicit
    return Quickshell.env("XDG_RUNTIME_DIR") + "/themesync.sock"
  }

  // The daemon's last answer to {"cmd":"status"} (its `info` object), or null.
  property var info: null
  property bool daemonUp: false
  // False until the first status call came back (with an answer or a refusal).
  property bool answered: false
  property bool refreshing: false
  // The action in flight ("sync", "push_list", "reset_counter") or "".
  property string acting: ""
  // What the last action said, shown under the buttons until the next one.
  property string actionText: ""

  readonly property bool paired: !!info && info.paired === true
  readonly property bool onAir: !!info && info.beacon === "on"
  readonly property bool locked: !!info && info.ctr_locked === true
  readonly property bool trouble: !!info && (info.ctr_locked === true || info.beacon === "off_air")
  readonly property bool hidden: hideWhenNoDaemon && !daemonUp

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // nf-md-watch; the same mark in the bar and in the panel's hero.
  readonly property string watchGlyph: "\u{f0589}"

  // The bar mark is drawn, not a font glyph: the icon glyph comes from whatever fallback
  // font serves the private-use codepoint, with side bearings no QML metric reports
  // reliably, and its ink sat ~1 px right of the slot-centred underline. Shapes are
  // centred by construction, at every theme font and scale.
  readonly property real openPanelIndicatorWidth: barMark.width

  visible: !hidden
  implicitWidth: hidden ? 0 : button.implicitWidth
  implicitHeight: button.implicitHeight

  function refresh() {
    if (!statusCall.start({ cmd: "status" }, 5000, root.applyStatus)) return
    refreshing = true
  }

  function open() {
    refresh()
    root.controller.show()
  }

  function applyStatus(raw) {
    refreshing = false
    answered = true
    var reply = root.parse(raw)
    if (!reply) {
      if (daemonUp) actionText = ""
      daemonUp = false
      info = null
      return
    }
    daemonUp = true
    info = reply.info || null
  }

  function parse(raw) {
    var text = String(raw || "").trim()
    if (text === "") return null
    try {
      return JSON.parse(text)
    } catch (e) {
      return null
    }
  }

  // Send the theme again ("sync"), push the theme list ("push_list") or forget the request
  // counter ("reset_counter"). A list push connects over GATT and can take a while, hence
  // the long wait; the daemon itself gives up at 90 s.
  function act(cmd) {
    if (acting !== "" || !daemonUp) return
    var req = { cmd: cmd }
    if (cmd === "push_list") req.force = true
    var waitMs = cmd === "push_list" ? 100000 : 30000
    if (!actionCall.start(req, waitMs, function(raw) { root.finishAction(cmd, raw) })) return
    acting = cmd
    actionText = cmd === "sync" ? "sending the theme…" : (cmd === "push_list" ? "pushing the theme list…" : "resetting the counter…")
  }

  function finishAction(cmd, raw) {
    acting = ""
    var reply = root.parse(raw)
    if (!reply) actionText = "no answer from the daemon"
    else actionText = (reply.ok ? "" : "failed: ") + String(reply.message || (reply.ok ? "done" : "error"))
    refresh()
  }

  function beaconLabel() {
    if (!info) return "—"
    switch (String(info.beacon)) {
      case "on": return "on the air"
      case "idle": return root.paired ? "idle, nothing to send" : "idle, no key"
      case "off_air": return "off the air"
      default: return String(info.beacon)
    }
  }

  function scanLabel() {
    if (!info) return "—"
    var s = String(info.scan)
    if (s === "on" && info.monitor === true) return "on, with the advertisement monitor"
    return s
  }

  function heroMeta() {
    if (!answered) return "asking the daemon…"
    if (!daemonUp) return "daemon not running"
    if (!info) return "daemon up"
    if (info.ctr_locked === true) return "counter locked — see below"
    if (info.beacon === "off_air") return "beacon off the air"
    return String(info.pairing) + " · beacon " + String(info.beacon)
  }

  // One request per connection, like the CLI: connect, write the line, read the reply,
  // the daemon closes. Two of these so a slow list push never blocks the status poll.
  component DaemonCall: Item {
    id: call
    property bool busy: false
    property var callback: null
    property string line: ""

    function start(request, timeoutMs, cb) {
      if (busy) return false
      busy = true
      callback = cb
      line = JSON.stringify(request) + "\n"
      guard.interval = timeoutMs
      guard.restart()
      sock.path = root.socketPath
      sock.connected = true
      return true
    }

    function finish(data) {
      if (!busy) return
      busy = false
      guard.stop()
      sock.connected = false
      var cb = callback
      callback = null
      line = ""
      if (cb) cb(data)
    }

    Socket {
      id: sock
      path: root.socketPath
      parser: SplitParser {
        onRead: function(data) { call.finish(data) }
      }
      onConnectedChanged: if (connected && call.line !== "") write(call.line)
      onError: function(err) { call.finish("") }
    }

    Timer {
      id: guard
      repeat: false
      onTriggered: call.finish("")
    }
  }

  DaemonCall { id: statusCall }
  DaemonCall { id: actionCall }

  Timer {
    interval: root.refreshMs
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  // While the panel is open the numbers should move with the watch, not with the bar's
  // relaxed cadence.
  Timer {
    interval: 2000
    running: root.opened
    repeat: true
    onTriggered: root.refresh()
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.watchGlyph
    fontSize: Style.font.icon
    labelVisible: false  // the WatchMark below draws the icon; text only sizes the slot
    active: root.trouble
    dimmed: !root.trouble && !(root.daemonUp && root.paired && root.onAir)
    tooltipText: ""

    WatchMark {
      id: barMark
      anchors.centerIn: parent
      color: button.active ? button.activeColor : button.foreground
    }

    onPressed: function(b) {
      if (b === Qt.RightButton) root.act("sync")
      else if (b === Qt.MiddleButton) root.refresh()
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(320))
    contentHeight: panel.fittedContentHeight(column.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onActivateRequested: root.refresh()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(t) {
        var k = String(t).toLowerCase()
        if (k === "r") root.refresh()
        else if (k === "s") root.act("sync")
        else if (k === "l") root.act("push_list")
        else if (k === "c" && root.locked) root.act("reset_counter")
      }

      Column {
        id: column
        width: parent.width
        spacing: Style.space(12)

        PanelHero {
          width: parent.width
          title: "Themesync"
          meta: root.heroMeta()
          detail: root.info ? String(root.info.theme || "") : ""
          foreground: root.foreground
          fontFamily: root.fontFamily

          iconComponent: Component {
            WatchMark {
              size: Style.font.display
              color: root.trouble ? root.urgent : root.foreground
            }
          }
        }

        PanelSeparator { foreground: root.foreground }

        Text {
          width: parent.width
          visible: !root.daemonUp
          text: "The daemon is not answering on " + root.socketPath + ".\nsystemctl --user start themesync · themesync doctor"
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Column {
          width: parent.width
          visible: !!root.info
          spacing: Style.space(6)

          PanelSectionHeader {
            width: parent.width
            text: "WATCH"
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          StatRow {
            label: "Pairing"
            value: root.info ? String(root.info.pairing) : ""
          }
          StatRow {
            label: "Address"
            value: root.info && root.info.watch ? String(root.info.watch) : "unknown until pairing"
          }
          StatRow {
            label: "Last request"
            value: root.info && root.info.last_request ? String(root.info.last_request) : "none yet"
          }
          StatRow {
            label: "Counter"
            value: root.info ? ("#" + root.info.ctr_last + (root.locked ? " · locked" : "")) : ""
            alarming: root.locked
          }
          StatRow {
            visible: !!root.info && Number(root.info.stale_rejected) > 0
            label: "Stale rejected"
            value: root.info ? String(root.info.stale_rejected) : ""
            alarming: true
          }
          StatRow {
            label: "Theme list"
            value: root.info ? String(root.info.list_push) : ""
          }
        }

        Column {
          width: parent.width
          visible: !!root.info
          spacing: Style.space(6)

          PanelSectionHeader {
            width: parent.width
            text: "DESKTOP"
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          StatRow {
            label: "Beacon"
            value: root.beaconLabel()
            alarming: !!root.info && root.info.beacon === "off_air"
          }
          StatRow {
            label: "Scan"
            value: root.scanLabel()
          }
          StatRow {
            label: "Theme hook"
            value: root.info ? (root.info.hook_installed === true ? "installed" : "missing — themesync install") : ""
            alarming: !!root.info && root.info.hook_installed !== true
          }
          StatRow {
            label: "Protocol"
            value: root.info ? String(root.info.protocol) : ""
          }
        }

        Text {
          width: parent.width
          visible: root.locked
          text: "The daemon refuses every request because the saved counter is unreadable or the watch started counting again (reflashed?). Reset it only for your own watch."
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        PanelSeparator {
          visible: root.daemonUp
          foreground: root.foreground
        }

        Flow {
          width: parent.width
          visible: root.daemonUp
          spacing: Style.space(8)

          Button {
            text: root.acting === "sync" ? "Sending…" : "Send theme"
            iconText: "\u{f048a}"  // nf-md-send
            tooltipText: "Read the current Omarchy theme again and put it on the air (also: right-click the mark, or S)"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: root.acting === ""
            onClicked: root.act("sync")
          }

          Button {
            text: root.acting === "push_list" ? "Pushing…" : "Push theme list"
            iconText: "\u{f0279}"  // nf-md-format_list_bulleted
            tooltipText: "Send the installed themes to the watch over GATT, even if it already has this list (L)"
            foreground: root.foreground
            fontFamily: root.fontFamily
            enabled: root.acting === "" && root.paired
            onClicked: root.act("push_list")
          }

          Button {
            visible: root.locked
            text: root.acting === "reset_counter" ? "Resetting…" : "Reset counter"
            iconText: "\u{f0450}"  // nf-md-refresh
            tooltipText: "Forget the last accepted request counter (C)"
            foreground: root.urgent
            fontFamily: root.fontFamily
            enabled: root.acting === ""
            onClicked: root.act("reset_counter")
          }
        }

        Text {
          width: parent.width
          visible: root.actionText !== ""
          text: root.actionText
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }
      }
    }
  }

  // A wristwatch, drawn: two strap stubs and a ring, the same proportions as nf-md-watch.
  component WatchMark: Item {
    id: mark
    property color color: root.foreground
    // Height; everything else derives from it. Follows the user's font scale.
    property real size: Style.spaceReal(15)

    implicitWidth: Math.round(size * 0.72)
    implicitHeight: Math.round(size)
    readonly property int strapWidth: Math.round(implicitWidth * 0.62)
    readonly property int strapHeight: Math.max(2, Math.round(size * 0.17))

    Behavior on color {
      enabled: !root.bar || root.bar.foregroundAnimationEnabled
      ColorAnimation { duration: 160 }
    }

    Rectangle {
      anchors.top: parent.top
      anchors.horizontalCenter: parent.horizontalCenter
      width: mark.strapWidth
      height: mark.strapHeight
      color: mark.color
    }

    Rectangle {
      anchors.bottom: parent.bottom
      anchors.horizontalCenter: parent.horizontalCenter
      width: mark.strapWidth
      height: mark.strapHeight
      color: mark.color
    }

    Rectangle {
      anchors.centerIn: parent
      width: mark.implicitWidth
      height: width
      radius: width / 2
      color: "transparent"
      border.color: mark.color
      border.width: Math.max(1.4, mark.size * 0.11)
    }
  }

  // One line of the state: what it is on the left, its value on the right.
  component StatRow: Item {
    id: row
    property string label: ""
    property string value: ""
    property bool alarming: false

    width: parent ? parent.width : 0
    implicitHeight: Math.max(labelText.implicitHeight, valueText.implicitHeight)

    Text {
      id: labelText
      text: row.label
      color: root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
    }

    Text {
      id: valueText
      text: row.value
      color: row.alarming ? root.urgent : root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.body
      anchors.left: labelText.right
      anchors.leftMargin: Style.spacing.md
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      horizontalAlignment: Text.AlignRight
      elide: Text.ElideMiddle
    }
  }
}
