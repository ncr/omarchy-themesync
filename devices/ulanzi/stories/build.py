#!/usr/bin/env python3
"""Build local story engine; pinned QuickJS 2026-06-04, Zig ARM musl cross target."""
from pathlib import Path
import argparse,subprocess,os,shutil
p=Path(__file__).resolve().parent
parser=argparse.ArgumentParser();parser.add_argument('--host',action='store_true');parser.add_argument('--quickjs',type=Path,default=os.environ.get('QUICKJS_SOURCE',p.parent/'quickjs-2026-06-04'));parser.add_argument('--zig',default=os.environ.get('ZIG') or shutil.which('zig') or str(p.parent/'toolchain/zig-x86_64-linux-0.17.0/zig'));args=parser.parse_args();q=args.quickjs.resolve()
common=['-O2','-D_GNU_SOURCE','-DCONFIG_VERSION="2026-06-04"','-I'+str(q),str(p/'storyd.c'),*[str(q/f) for f in ('quickjs.c','dtoa.c','libregexp.c','libunicode.c','cutils.c')],'-lm','-lpthread']
output=p/('storyd-host' if args.host else 'storyd')
cmd=([os.environ.get('CC','cc')] if args.host else [args.zig,'cc','-target','arm-linux-musleabihf','-mcpu=cortex_a7','-static','-s'])+common+['-o',str(output)]
subprocess.run(cmd,check=True,env=dict(os.environ,ZIG_GLOBAL_CACHE_DIR=os.environ.get('ZIG_GLOBAL_CACHE_DIR',str(p/'zig-cache'))))
print('Built',output)
