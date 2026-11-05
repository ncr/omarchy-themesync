import base64,json,subprocess,sys,tempfile
from pathlib import Path
p=Path(__file__).resolve().parent
with tempfile.TemporaryDirectory() as tmp:
    subprocess.run([sys.executable,str(p.parent/'sounds/make_sounds.py'),tmp],check=True,capture_output=True)
    sounds={w.stem:base64.b64encode(w.read_bytes()).decode() for w in sorted(Path(tmp).glob('*.wav'))}
html=(p/'gallery.template.html').read_text().replace('/* ORDERS_SOURCE */',(p.parent/'stories/renderer.js').read_text()).replace('/* SOUNDS */{}',json.dumps(sounds))
(p/'index.html').write_text(html)
print('Built',p/'index.html')
