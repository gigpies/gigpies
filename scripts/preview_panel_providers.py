#!/usr/bin/env python3
"""Explicit real-owner previews: private synthetic services, no physical devices.
Requires already built canonical provider/consumer binaries. Creates owned ignored
artifacts only, never installs/activates a service or grants writer authority.
"""
import subprocess,time,os,json,tempfile,signal
import socket as sockets
from pathlib import Path
root=Path(__file__).resolve().parents[1];base=root/'artifacts/instrument-panel';show='11111111-1111-4111-8111-111111111111'
import argparse
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('providers',nargs='*',choices=['desk','lightdesk'])
parser.add_argument('--cpu-terminal',action='store_true')
parser.add_argument('--spectrum',action='store_true',help='Explicit deterministic software PCM through real spectrum provider')
parser.add_argument('--spectrum-hot',action='store_true',help='Deterministic -1 dBFS input through real processing; software evidence only')
parser.add_argument('--materials',action='store_true',help='Channel-only cached material experiment')
parser.add_argument('--disconnect',action='store_true')
parser.add_argument('--channel',action='store_true',help='Desk channel page with explicit injected software role; no devices')
parser.add_argument('--mix',action='store_true',help='Desk Mix page, injected software role, 16-input deterministic software audio; no devices')
parser.add_argument('--keys',default='')
parser.add_argument('--resize')
parser.add_argument('--click')
parser.add_argument('--fallback',action='store_true')
parser.add_argument('--suffix',default='production')
parser.add_argument('--font-size',type=int,choices=[16,20,24])
args=parser.parse_args()
names=args.providers or ['desk','lightdesk']
if args.materials:os.environ['SHR_CHANNEL_MATERIALS']='rio-kitty-v1'
for name in names:
 scratch=tempfile.TemporaryDirectory(prefix='shr-panel-owner-');directory=Path(scratch.name)
 if name=='desk':
  identity=directory/'audio.identity';epoch=str(int(json.loads(identity.read_text())['high_epoch'])+1) if identity.exists() else '1'
  provider=[str(root/'target/release/gigpies-headless'),'--directory',str(directory),'--show',show,'--epoch',epoch,'--ticks','60000']
  if args.mix:provider=[str(root/'target/release/examples/spectrum-source'),str(directory),show,epoch,'mix']
  elif args.spectrum or args.spectrum_hot:provider=[str(root/'target/debug/examples/spectrum-source'),str(directory),show,epoch,'hot' if args.spectrum_hot else 'tones']
  socket=directory/'audio.sock'
  app=[str(root.parent/'shr-desk/target/debug/shr-desk'),'--terminal',str(socket),show,epoch,'panel-preview','foh','--processing']
  if args.mix:app.append('--dynamic')
  if args.channel or args.mix:
   role_dir=directory/'role';role_dir.mkdir(exist_ok=True,mode=0o700)
   acquire=json.loads((root/'tests/fixtures/gp09/v1/acquire-a.json').read_text())
   registry=role_dir/'registry.json'
   if registry.exists():acquire['expected_generation']=json.loads(registry.read_text())['generation']
   role_file=directory/'role-acquire.json';role_file.write_text(json.dumps(acquire));role_file.chmod(0o600)
   app+=['--role-provider',str(root/'target/release/gigpies-role-lease'),str(role_dir),str(role_file)]
 else:
  provider=[str(root.parent/'shr-lux/target/release/lux-service'),'--synthetic-private-dir',str(directory)]
  socket=directory/'lux.sock'
  app=[str(root.parent/'shr-lightdesk/target/debug/shr-lightdesk'),'terminal','--socket-path',str(socket),'--show-id',show,'--epoch','1']
 capture=None
 with (directory/'provider.log').open('w') as log:
  p=subprocess.Popen(provider,stdout=log,stderr=log)
  try:
   for _ in range(100):
    if p.poll() is not None:raise RuntimeError('provider exited')
    try:
     with sockets.socket(sockets.AF_UNIX,sockets.SOCK_STREAM) as probe:
      probe.settimeout(.2);probe.connect(str(socket))
     break
    except OSError:time.sleep(.05)
   else:raise RuntimeError('private provider readiness deadline')
   if name=='lightdesk':
    identity=json.loads((directory/'lighting-epoch.json').read_text())
    assert identity['show_id']==show
    app[-1]=str(identity['epoch'])
   preview=['/usr/bin/python3',str(root/'scripts/preview_panel.py'),'--name',name+'-'+args.suffix,'--font-size','24' if args.channel or args.mix else '20','--width','1928' if args.channel or args.mix else '1640','--height','1218' if args.mix else '1122' if args.channel else '950','--keys',('F2,'+args.keys).rstrip(',') if args.channel and name=='desk' else args.keys]
   if args.font_size:
    preview+=['--font-size',str(args.font_size),'--width',str(80*args.font_size+8),'--height',str((49 if args.mix else 45)*args.font_size+42)]
   if args.cpu_terminal:preview+=['--cpu-terminal']
   if args.resize:preview+=['--resize',args.resize]
   if args.click:preview+=['--click',args.click]
   if args.fallback:preview+=['--family','DejaVu Sans Mono','--system-fonts-only','--font-size','20']
   ready=directory/'preview-ready'
   if args.disconnect:preview+=['--settle-seconds','3','--ready-file',str(ready)]
   elif args.mix:preview+=['--settle-seconds','2.5']
   capture=subprocess.Popen([*preview,'--command',*app])
   if args.disconnect:
    deadline=time.monotonic()+20
    while not ready.exists():
     if capture.poll() is not None or time.monotonic()>deadline:raise RuntimeError('preview readiness deadline')
     time.sleep(.02)
    p.terminate();p.wait(timeout=5)
   if capture.wait(timeout=55):raise RuntimeError('preview failed')
   if args.channel and name=='desk' and not args.disconnect:
    request=json.loads((root.parent/'shr-desk/tests/fixtures/gp07/v2/snapshot-request.json').read_text())
    request['epoch']=epoch
    payload=json.dumps(request).encode()
    with sockets.socket(sockets.AF_UNIX,sockets.SOCK_STREAM) as reader:
     reader.settimeout(2);reader.connect(str(socket));reader.sendall(len(payload).to_bytes(4,'big')+payload)
     def exact(n):
      data=b''
      while len(data)<n:
       part=reader.recv(n-len(data))
       if not part:raise RuntimeError('short owner readback')
       data+=part
      return data
     length=int.from_bytes(exact(4),'big');assert length<=65536
     reply=json.loads(exact(length))
    observed=reply['snapshot'];channel=observed['channels'][0]
    print('Independent owner readback:',json.dumps({'revision':observed['revision'],'ready':channel['ready'],'eq_bypass':channel['current']['eq_bypass'],'band1_gain_mdb':channel['current']['band1_gain_mdb'],'compressor_bypass':channel['current']['compressor_bypass'],'ratio_milli':channel['current']['ratio_milli']}))
  finally:
   if capture is not None and capture.poll() is None:
    capture.send_signal(signal.SIGINT)
    try:capture.wait(timeout=10)
    except subprocess.TimeoutExpired:capture.kill();capture.wait(timeout=5)
   p.terminate();p.wait(timeout=5)
   scratch.cleanup()
