from pathlib import Path
p=Path(__file__).resolve().parent
(p/'index.html').write_text((p/'gallery.template.html').read_text().replace('/* DEMOS_SOURCE */',(p.parent/'stories/demos.js').read_text()))
print('Built',p/'index.html')
