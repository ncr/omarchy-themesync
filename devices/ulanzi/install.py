#!/usr/bin/env python3
"""Install the Ulanzi bridge and clock files from this checkout into one fixed place, so the running services
never execute from a working copy. Tests first; then copy, point the config and the user unit there, restart
the bridge. --device also uploads the clock files and arrival sounds and restarts the scene engine on the TC002.

  python3 devices/ulanzi/install.py [--device] [--storyd PATH] [--skip-tests]
"""
import argparse,json,os,shutil,subprocess,sys,urllib.error,urllib.request
from pathlib import Path
REPO=Path(__file__).resolve().parents[2];STORIES=REPO/'devices/ulanzi/stories'
HOME=Path.home();DEST=HOME/'.local/share/themesync/ulanzi';CONFIG=HOME/'.config/themesync/paid-orders.json'
UNIT='themesync-paid-orders.service';UNIT_DIR=HOME/'.config/systemd/user'
FILES=['paid_orders.py','ensure_runtime.py','renderer.js','demos.js','device.js']

def run(cmd,**kw):
    print('+',' '.join(map(str,cmd)),flush=True);subprocess.run(cmd,check=True,**kw)
class Clock:
    """The runtime's own API with the admin token, used only here: uploads, the sound token, the speaker setting."""
    def __init__(self,config):
        self.host=config['device_host'];tokens=(Path(config['clock_package'])/'tokens').read_text().splitlines()
        self.token=next(l.split('=',1)[1] for l in tokens if l.startswith('admin='))
    def call(self,method,path,body=None,raw=None):
        data=raw if raw is not None else json.dumps(body).encode() if body is not None else None
        r=urllib.request.Request(f'http://{self.host}/api/v1{path}',data=data,method=method,headers={'Authorization':'Bearer '+self.token,'Content-Type':'application/octet-stream' if raw is not None else 'application/json'})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(r,timeout=10) as resp:return json.load(resp)
    def upload(self,name,data):
        for off in range(0,len(data),4096):
            chunk=data[off:off+4096];self.call('PUT',f'/sounds/{name}?offset={off}&final={1 if off+len(chunk)>=len(data) else 0}',raw=chunk)

def sounds(config):
    # The scene engine plays with its own token that holds only the sound scope; uploads stay here.
    clock=Clock(config);run([sys.executable,REPO/'devices/ulanzi/sounds/make_sounds.py',DEST/'sounds'])
    if not clock.call('GET','/config')['sound']['enabled']:clock.call('PATCH','/config',{'sound_enabled':True})
    stored={s['name'] for s in clock.call('GET','/sounds')['sounds']}
    for old in stored:clock.call('DELETE','/sounds/'+old)  # replace all: overwriting in place could pass the 256 KiB budget
    for wav in sorted((DEST/'sounds').glob('*.wav')):clock.upload(wav.stem,wav.read_bytes())
    token=DEST/'sound.token'
    if not token.exists():
        known={c['name'] for c in clock.call('GET','/tokens')['clients']}
        issued=clock.call('POST','/tokens/stories-sound/rotate') if 'stories-sound' in known else clock.call('POST','/tokens',{'name':'stories-sound','scopes':['sound']})
        fd=os.open(token,os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
        with os.fdopen(fd,'w') as f:f.write(issued['token']+'\n')
    print('Sounds:',', '.join(s['name'] for s in clock.call('GET','/sounds')['sounds']))

def place(src,dst,mode=0o644):
    tmp=dst.with_name(dst.name+'.new');shutil.copyfile(src,tmp);os.chmod(tmp,mode);os.replace(tmp,dst)

def main():
    a=argparse.ArgumentParser(description=__doc__,formatter_class=argparse.RawDescriptionHelpFormatter)
    a.add_argument('--device',action='store_true',help='also upload to the clock and restart its scene engine')
    a.add_argument('--storyd',type=Path,help='storyd ARM binary (default: built one in the checkout, else keep the installed one)')
    a.add_argument('--skip-tests',action='store_true');args=a.parse_args()
    if not args.skip_tests:
        for t in ('test-orders.cjs','test-device.cjs','test-compo.cjs'):run(['node',STORIES/t])
        run([sys.executable,STORIES/'test-paid-orders.py'])
    config=json.loads(CONFIG.read_text())
    DEST.mkdir(parents=True,exist_ok=True)
    for name in FILES:place(STORIES/name,DEST/name)
    # storyd is built, not tracked: take a fresh build, else keep what is installed, else the old story package's.
    storyd=args.storyd or (STORIES/'storyd' if (STORIES/'storyd').exists() else None)
    if storyd is None and not (DEST/'storyd').exists():storyd=Path(config['story_package'])/'storyd'
    if storyd:place(storyd,DEST/'storyd',0o755)
    if not (DEST/'storyd').exists():sys.exit('no storyd binary: build one with stories/build.py or pass --storyd')
    config['story_package']=str(DEST);config['runtime_recovery']=[sys.executable,str(DEST/'ensure_runtime.py'),'--config',str(CONFIG)]
    tmp=CONFIG.with_suffix('.json.new');tmp.write_text(json.dumps(config,indent=2)+'\n');os.chmod(tmp,0o600);os.replace(tmp,CONFIG)
    place(REPO/'systemd'/UNIT,UNIT_DIR/UNIT)
    run(['systemctl','--user','daemon-reload']);run(['systemctl','--user','restart',UNIT])
    if args.device:sounds(config);run([sys.executable,DEST/'ensure_runtime.py','--config',CONFIG])
    print(f'Installed into {DEST}; {UNIT} restarted'+(', clock updated.' if args.device else '.'))
if __name__=='__main__':main()
