from pathlib import Path
import json,base64,sys
from PIL import Image,ImageDraw,ImageFont
out=Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).resolve().parent/'previews'
metas=json.loads((out/'catalogue.json').read_text());sheet=Image.new('RGB',(1120,5*240+32),'#0c1019');draw=ImageDraw.Draw(sheet)
for i,m in enumerate(metas):
 data=json.loads((out/(m['id']+'.json')).read_text());frames=[Image.frombytes('RGB',(52,16),base64.b64decode(x)) for x in data['frames']];pose=frames[int(min(5,m['duration']/3)*8)];pose.save(out/(m['id']+'-52x16.png'))
 x=16+(i%4)*280;y=16+(i//4)*240;draw.text((x,y),str(i+1).zfill(2)+' / '+m['name'],fill='#dfecff');sheet.paste(pose.resize((260,80),Image.Resampling.NEAREST),(x,y+22));sheet.paste(frames[int(m['duration']*.65*8)].resize((260,80),Image.Resampling.NEAREST),(x,y+118))
 # Shareable real animated previews, indexed palette per frame, 8 fps.
 gif=[im.resize((520,160),Image.Resampling.NEAREST).convert('P',palette=Image.Palette.ADAPTIVE,colors=24) for im in frames]
 gif[0].save(out/(m['id']+'.gif'),save_all=True,append_images=gif[1:],duration=125,loop=0,disposal=2,optimize=False)
sheet.save(out/'twenty-demos.png');print('20 full-length GIFs and native-resolution contact sheet exported.')
