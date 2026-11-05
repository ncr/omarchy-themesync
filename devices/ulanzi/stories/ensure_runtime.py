#!/usr/bin/env python3
"""Re-establish the RAM runtime after a power cycle. Never changes device boot/init/firmware."""
import argparse,json,os,subprocess,urllib.request
from pathlib import Path

def main():
 a=argparse.ArgumentParser();a.add_argument('--config',required=True,type=Path);args=a.parse_args();c=json.loads(args.config.read_text())
 package=Path(c['clock_package']);stories=Path(c['story_package']);host=c['device_host'];env=dict(os.environ,ANDROID_USER_HOME=str(package/'.android'))
 def run(command,timeout=45):
  r=subprocess.run(command,env=env,capture_output=True,text=True,timeout=timeout)
  if r.returncode:raise RuntimeError('runtime recovery command failed')
  return r.stdout
 def adb(*args):return run([str(package/'bin/adb'),'-P','5039','-s',host+':5555',*args])
 def shell(command):return adb('shell',command)
 # A healthy runtime needs no bootstrap upload (important for the old ADB sync server).
 healthy=False
 try:
  token=next(line[8:] for line in (package/'tokens').read_text().splitlines() if line.startswith('control='))
  request=urllib.request.Request('http://'+host+'/api/v1/status',headers={'Authorization':'Bearer '+token})
  with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request,timeout=3) as response:healthy=json.load(response).get('renderer')=='running'
 except (OSError,ValueError,StopIteration):pass
 if not healthy:run(['/usr/bin/python3',str(package/'runtime-control.py'),'start','--host',host],timeout=180)
 shell('/tmp/busybox mkdir -p /tmp/tc002/stories /data/tc002/stories; /tmp/busybox chmod 700 /data/tc002/stories')
 # Upload from the installed host bundle, so a recovered device gets the tested build.
 for name in ('storyd','renderer.js','demos.js','device.js'):adb('push',str(stories/name),'/tmp/tc002/stories/'+name)
 shell('/tmp/busybox cp /tmp/tc002/stories/storyd /tmp/tc002/stories/renderer.js /tmp/tc002/stories/demos.js /tmp/tc002/stories/device.js /data/tc002/stories/')
 # The sound-only token lives beside the engine's state; without it the scenes stay silent.
 if (stories/'sound.token').exists():adb('push',str(stories/'sound.token'),'/data/tc002/stories/sound.token');shell('/tmp/busybox chmod 600 /data/tc002/stories/sound.token')
 shell('if [ -f /tmp/tc002/stories.pid ]; then kill -TERM $(cat /tmp/tc002/stories.pid) 2>/dev/null; /tmp/busybox sleep 1; fi; /tmp/busybox chmod 700 /tmp/tc002/stories/storyd; trap "" HUP; /tmp/tc002/stories/storyd /tmp/tc002/stories/renderer.js /tmp/tc002/stories/device.js /data/tc002/state/credentials/tokens /data/tc002/stories/state.json </dev/null >/tmp/tc002/stories.log 2>&1 & echo $! >/tmp/tc002/stories.pid')
 print('Ulanzi RAM scene engine restored.',flush=True)
if __name__=='__main__':main()
