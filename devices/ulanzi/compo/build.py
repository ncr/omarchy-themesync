from pathlib import Path
p=Path(__file__).resolve().parent;stories=p.parent/'stories'
html=(p/'gallery.template.html').read_text().replace('/* RENDERER_SOURCE */',(stories/'renderer.js').read_text()).replace('/* DEMOS_SOURCE */',(stories/'demos.js').read_text())
(p/'index.html').write_text(html)
print('Built',p/'index.html')
