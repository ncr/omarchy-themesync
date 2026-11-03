# Quiet Garden, 52x16. Native sprites: three 16x16 landscape tiles and five 8x8 plants.
# Physical controls: encoder (knob), then '-' (left), 'Ulanzi' (middle), '+' (right).
# Middle or encoder press toggles garden/clock. Minus/plus adjust brightness.
# Encoder: native clock faces on the clock, butterfly speed in the garden.
# Hold the encoder for the runtime's normal device menu.
import time

var g_active = true
var g_epoch = time.time()
var g_paid = 0
var g_season = 2
var g_accent = 0x4cc9ff
var g_secondary = 0xa78bfa
var g_leaf = 0x71b66b
var g_brightness = 35
var g_tick = 0
var g_wind = 2
var g_celebrate_until = 0
var g_restore_clock = false
# One row of five distinct beds: plants never overlap each other or the tree.
var g_places = [[11,7],[19,7],[27,7],[35,7],[43,7]]

def garden_show()
  g_active = true
  g_celebrate_until = 0
  tc002.scene('canvas')
end

def garden_clock()
  g_active = false
  g_celebrate_until = 0
  tc002.scene('clock')
end

def garden_bloom()
  g_restore_clock = !g_active
  g_active = true
  g_celebrate_until = time.time() + 8
  tc002.scene('canvas')
end

def garden_draw()
  var age = time.time() - g_epoch
  if age < 0 age = 0 end
  # A seedling every half hour, a flower every six hours, plus a flower per paid order.
  var count = 2 + int(age / 1800) + g_paid
  if count > 5 count = 5 end
  var flowers = int(age / 21600) + g_paid
  if flowers > count flowers = count end
  panel.clear()
  panel.sprite(0,0,'garden0')
  panel.sprite(16,0,'garden1')
  panel.sprite(32,0,'garden2')
  # Use all 52 columns. Sky is deliberately unlit; the ground anchors the silhouettes.
  var soil = 0x005000
  if g_season == 2 soil = 0x804000
  elif g_season == 3 soil = 0x006080 end
  panel.rect(0,15,52,1,soil,1)
  var ground = 0
  while ground < 52
    panel.rect(ground,14,2,1,soil,1)
    ground += 8
  end
  var i = 0
  while i < count
    var p = g_places[i]
    var id = 'shoot'
    if i < flowers
      var kind = (i + g_season + int(g_paid / 5)) % 3
      if kind == 0 id = 'flower0'
      elif kind == 1 id = 'flower1'
      else id = 'flower2' end
    end
    panel.sprite(p[0],p[1],id)
    i += 1
  end
  # A butterfly makes a slow, irregular pass. Most of the time the garden is still.
  var flight = int(g_tick * g_wind / 3) % 180
  if flight < 58
    var y = 1 + int(g_tick / 4) % 2
    panel.sprite(flight-7,y,'moth')
  end
  if g_celebrate_until > time.time()
    var position = g_paid - 1
    if position < 0 position = 0 end
    position = position % 5
    var p = g_places[position]
    if g_tick % 2 == 0
      panel.pixel(p[0]+3,p[1]-1,0xffedaf)
      panel.pixel(p[0]+1,p[1]+1,g_accent)
      panel.pixel(p[0]+6,p[1]+2,g_secondary)
    end
  end
  panel.push()
end

tc002.on('button',def(control,event,steps)
  if event == 'press'
    if control == 'middle' || control == 'knob'
      if g_active garden_clock() else garden_show() end
    elif control == 'left' || control == 'right'
      if control == 'left' g_brightness -= 5 else g_brightness += 5 end
      if g_brightness < 5 g_brightness = 5 end
      if g_brightness > 75 g_brightness = 75 end
      tc002.brightness(g_brightness)
      # The factory button arbiter selects a base first; restore the app the user was using.
      if g_active tc002.scene('canvas') else tc002.scene('clock') end
    end
  elif control == 'rotary' && g_active
    if event == 'cw' g_wind += 1 elif event == 'ccw' g_wind -= 1 end
    if g_wind < 1 g_wind = 1 end
    if g_wind > 5 g_wind = 5 end
  end
end)

tc002.every(200,def()
  g_tick += 1
  if g_celebrate_until > 0 && time.time() >= g_celebrate_until
    g_celebrate_until = 0
    if g_restore_clock garden_clock() end
  end
  if g_active garden_draw() end
end)

garden_show()
print('Quiet Garden ready: encoder / - / Ulanzi / +')
