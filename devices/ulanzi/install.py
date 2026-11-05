#!/usr/bin/env python3
"""Install the Ulanzi bridge and clock files from this checkout into one fixed place, so the running services
never execute from a working copy. Tests first; then copy, point the config and the user unit there, restart
the bridge. --device also uploads the clock files and restarts the scene engine on the TC002.

  python3 devices/ulanzi/install.py [--device] [--storyd PATH] [--skip-tests]
"""
import argparse,json,os,shutil,subprocess,sys
from pathlib import Path
REPO=Path(__file__).resolve().parents[2];STORIES=REPO/'devices/ulanzi/stories'
HOME=Path.home();DEST=HOME/'.local/share/themesync/ulanzi';CONFIG=HOME/'.config/themesync/paid-orders.json'
UNIT='themesync-paid-orders.service';UNIT_DIR=HOME/'.config/systemd/user'
FILES=['paid_orders.py','ensure_runtime.py','renderer.js','demos.js','device.js']

def run(cmd,**kw):
    print('+',' '.join(map(str,cmd)),flush=True);subprocess.run(cmd,check=True,**kw)
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
    if args.device:run([sys.executable,DEST/'ensure_runtime.py','--config',CONFIG])
    print(f'Installed into {DEST}; {UNIT} restarted'+(', clock updated.' if args.device else '.'))
if __name__=='__main__':main()
