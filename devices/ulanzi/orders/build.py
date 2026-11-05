from pathlib import Path
p=Path(__file__).resolve().parent
(p/'index.html').write_text((p/'gallery.template.html').read_text().replace('/* ORDERS_SOURCE */',(p.parent/'stories/renderer.js').read_text()))
print('Built',p/'index.html')
