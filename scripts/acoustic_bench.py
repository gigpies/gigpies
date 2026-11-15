#!/usr/bin/env python3
"""Bounded two-node WAV acquisition; no PA analysis or automatic correction.

prepare/inspect are offline. capture requires explicit session authorization.
See docs/development/ACOUSTIC_BENCH.md.
"""
import argparse
import array
import hashlib
import json
import math
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import time
import wave

RATE = 48000
MAX_SECONDS = 20
PEAK_LIMIT = 10 ** (-30 / 20)


def read_pcm(path, max_seconds=MAX_SECONDS):
    with wave.open(str(path), 'rb') as w:
        if (w.getframerate(), w.getsampwidth(), w.getnchannels()) != (RATE, 2, 2):
            raise ValueError('Require stereo PCM16 WAV at 48000 Hz')
        if not 0 < w.getnframes() <= RATE * max_seconds:
            raise ValueError(f'Require 0 < duration <= {max_seconds} seconds')
        frames = w.getnframes()
        data = w.readframes(frames)
        if len(data) != frames * 4:
            raise ValueError('Truncated WAV')
    samples = array.array('h', data)
    if sys.byteorder != 'little':
        samples.byteswap()
    return samples


def db(value):
    return 20 * math.log10(value) if value else None


def summary(path, max_seconds=MAX_SECONDS):
    samples = read_pcm(path, max_seconds)
    channels = []
    for index in range(2):
        values = samples[index::2]
        channels.append({'peak_dbfs': db(max(abs(x) for x in values) / 32768),
                         'rms_dbfs': db(math.sqrt(sum(x*x for x in values) / len(values)) / 32768),
                         'full_scale_samples': sum(abs(x) >= 32767 for x in values)})
    return {'frames': len(samples)//2, 'channels': channels,
            'channels_bit_identical': samples[::2] == samples[1::2],
            'sha256': hashlib.sha256(Path(path).read_bytes()).hexdigest(),
            'calibration': None, 'common_clock': False,
            'absolute_latency_qualified': False}


def validate_playback(path, channel):
    samples = read_pcm(path)
    active = 0 if channel == 'left' else 1
    if any(samples[1-active::2]):
        raise ValueError('Unselected output must be exactly silent')
    peak = max(abs(x) for x in samples) / 32768
    if not 0 < peak <= PEAK_LIMIT:
        raise ValueError('Playback must be nonzero and peak <= -30 dBFS')
    if len(samples)//2 > RATE * 12:
        raise ValueError('Playback must be <= 12 seconds')
    return len(samples)//2 / RATE


def run(command, **kwargs):
    return subprocess.run(command, check=True, timeout=30, **kwargs)


def prepare(directory, channel, level):
    if not math.isfinite(level) or not -60 <= level <= -30:
        raise ValueError('Peak level must be -60..-30 dBFS')
    directory.mkdir(parents=True, exist_ok=False)
    # SoX owns excitation generation; these are exploratory signals, not SHR PA sweep records.
    effects = {'pilot': ['synth', '1', 'sine', '1000'],
               'sweep': ['synth', '6', 'sine', '80/12000'],
               'pink': ['synth', '6', 'pinknoise', 'highpass', '80', 'lowpass', '12000'],
               'burst': ['synth', '0.05', 'sine', '1000']}
    for name, effect in effects.items():
        duration = {'pilot': '1', 'sweep': '6', 'pink': '6', 'burst': '0.05'}[name]
        run(['sox', '-D', '-n', '-r', str(RATE), '-b', '16', '-c', '2',
             str(directory / (name + '.wav')), *effect,
             'fade', 't', '0.01', duration, '0.01', 'gain', '-n', str(level),
             'remix', '1' if channel == 'left' else '0',
             '0' if channel == 'left' else '1', 'pad', '1', '2'])
        validate_playback(directory / (name + '.wav'), channel)
    (directory / 'signals.json').write_text(json.dumps(
        {'channel': channel, 'peak_dbfs': level, 'rate': RATE,
         'purpose': 'Exploratory uncalibrated bench; not owner sweep evidence'}, indent=2)+'\n')


def capture(args):
    if not args.authorize_audio:
        raise ValueError('capture requires --authorize-audio for the current attended session')
    if not re.fullmatch(r'hw:[A-Za-z0-9_]+,[0-9]+', args.capture_device):
        raise ValueError('Use an explicit hw:CARD,DEVICE capture endpoint')
    duration = 8 if args.play is None else math.ceil(validate_playback(args.play, args.channel)) + 6
    if args.play and not re.fullmatch(r'hw:[A-Za-z0-9_]+,[0-9]+', args.playback_device):
        raise ValueError('Use an explicit hw:CARD,DEVICE playback endpoint')
    args.output.mkdir(parents=True, exist_ok=False)
    record = {'capture_device': args.capture_device, 'playback_device': args.playback_device,
              'channel': args.channel, 'state': 'incomplete', 'seconds': duration,
              'clock_domains': ['E205U USB', 'Pi4 USB HIFI AUDIO'],
              'timing': 'independent device clocks; SSH start is not a latency reference',
              'stimulus': summary(args.play) if args.play else None}
    manifest = args.output / 'session.json'
    manifest.write_text(json.dumps(record, indent=2)+'\n')
    pcm = args.output / 'capture.pcm'
    child = None
    try:
        with (args.output / 'capture.log').open('wb') as log:
            child = subprocess.Popen(['arecord', '-D', args.capture_device, '-t', 'raw',
                '-f', 'S16_LE', '-r', str(RATE), '-c', '2', '-d', str(duration), str(pcm)], stderr=log)
            deadline = time.monotonic() + 3
            while not pcm.exists() or pcm.stat().st_size < RATE * 4 // 4:
                if child.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('Capture did not become ready; no playback started')
                time.sleep(0.05)
            if args.play:
                # Fixed host; validated device; no remote files or persistent player.
                command = f'timeout --signal=TERM --kill-after=1 15 aplay -D {args.playback_device} -t wav'
                with args.play.open('rb') as source, (args.output / 'playback.log').open('wb') as log:
                    run(['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5',
                         'gigpies-pi4', command], stdin=source, stdout=log, stderr=log)
            if child.wait(timeout=duration+3) != 0:
                raise RuntimeError('Capture failed; inspect capture.log')
        logs = ''.join(p.read_text(errors='replace') for p in args.output.glob('*.log'))
        if re.search(r'overrun|underrun|xrun|error', logs, re.I):
            raise RuntimeError('Audio fault in logs; record remains incomplete')
        if pcm.stat().st_size != duration * RATE * 4:
            raise RuntimeError('Wrong capture length')
        with wave.open(str(args.output / 'capture.wav'), 'wb') as w:
            w.setparams((2, 2, RATE, 0, 'NONE', 'not compressed'))
            w.writeframes(pcm.read_bytes())
        pcm.unlink()
        record.update(state='complete', capture=summary(args.output / 'capture.wav'))
    finally:
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        manifest.write_text(json.dumps(record, indent=2)+'\n')


def live(args):
    """One finite real provider/Brain run. All configs/artifacts prepared beforehand."""
    if not args.authorize_audio:
        raise ValueError('live requires --authorize-audio for this session')
    if not 8 <= args.seconds <= 60:
        raise ValueError('Live capture must be 8..60 seconds')
    for path in (args.provider, args.binary, args.ready):
        if not path.is_absolute():
            raise ValueError('Remote paths must be absolute')
    # fuser checks PCM only; control-only PipeWire is not an audio owner.
    for command in (['sh', '-c', 'fuser /dev/snd/pcm*'],
                    ['ssh', 'gigpies-pi4', 'fuser /dev/snd/pcm*']):
        check = subprocess.run(command, capture_output=True, timeout=10)
        if check.returncode != 1 or check.stdout.strip():
            raise RuntimeError('PCM owner active or ownership check failed')
    args.output.mkdir(parents=True, exist_ok=False)
    record = {'state': 'incomplete', 'common_clock': False,
              'microphone_channel': 1, 'capture_channels': 2,
              'microphone_calibration': None, 'seconds': args.seconds,
              'provider': str(args.provider), 'brain': str(args.brain),
              'control': str(args.control), 'pilot_dbfs': -80,
              'pilot_hz': 440, 'warmup_minimum_seconds': 4}
    previous_term = signal.getsignal(signal.SIGTERM)
    def cancel(signum, frame):
        raise KeyboardInterrupt('bounded acoustic acquisition cancelled')
    signal.signal(signal.SIGTERM, cancel)
    children = []
    logs = []
    remote_pgid = None
    started = time.monotonic()
    def start(command, name, **kw):
        log = (args.output / (name+'.log')).open('wb'); logs.append(log)
        child = subprocess.Popen(command, stderr=log, **kw)
        children.append(child)
        return child
    try:
        pcm = args.output / 'capture.pcm'
        cap = start(['arecord', '-D', 'hw:E205U,0', '-t', 'raw', '-f', 'S16_LE',
                     '-r', str(RATE), '-c', '2', '-d', str(args.seconds), str(pcm)], 'capture')
        deadline = time.monotonic()+3
        while not pcm.exists() or pcm.stat().st_size < RATE:
            if cap.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError('Capture readiness failed')
            time.sleep(.025)
        # Timeout belongs to this process group only, survives SSH interruption.
        code = ("import subprocess; p=subprocess.Popen(" + repr([
            'timeout', '--signal=TERM', '--kill-after=2', str(args.seconds),
            str(args.binary), '--config', str(args.provider), '--activate-physical']) +
            ",start_new_session=True); print(p.pid,flush=True); raise SystemExit(p.wait())")
        owner = start(['ssh', 'gigpies-pi4', 'python3 -c '+shlex.quote(code)],
                      'provider', stdout=subprocess.PIPE, text=True)
        # Remote launch is bounded by SSH/remote timeout; no endless owner.
        import selectors
        with selectors.DefaultSelector() as selector:
            selector.register(owner.stdout, selectors.EVENT_READ)
            if not selector.select(5):
                raise RuntimeError('Remote launch deadline')
            remote_pgid = int(owner.stdout.readline())
        record['remote_pgid'] = remote_pgid
        deadline = time.monotonic()+5
        while True:
            ready = subprocess.run(['ssh', 'gigpies-pi4', 'test -f '+shlex.quote(str(args.ready))], timeout=5)
            if ready.returncode == 0:
                break
            if owner.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError('Provider readiness failed')
            time.sleep(.05)
        brain = start([str(args.local_binary), '--config', str(args.brain)], 'brain', stdout=subprocess.DEVNULL)
        time.sleep(.3)
        control = start([str(args.local_binary), '--config', str(args.control), '--activate-physical'],
                        'control', stdout=subprocess.DEVNULL)
        if control.wait(timeout=8) != 0:
            raise RuntimeError('Control failed')
        control_config = json.loads(args.control.read_text())['config']
        control_report = json.loads(Path(control_config['report']).read_text())
        if control_report.get('fault') is not None:
            raise RuntimeError('Review/apply/rearm refused: '+str(control_report['fault']))
        if owner.wait(timeout=args.seconds+2) != 0 or cap.wait(timeout=5) != 0:
            raise RuntimeError('Provider/capture failed')
        if brain.wait(timeout=8) != 0:
            raise RuntimeError('Brain failed')
        for log in logs:
            log.flush()
        text = ''.join(p.read_text(errors='replace') for p in args.output.glob('*.log'))
        if re.search(r'overrun|underrun|xrun', text, re.I):
            raise RuntimeError('Reported audio xrun')
        if pcm.stat().st_size != args.seconds*RATE*4:
            raise RuntimeError('Wrong capture length')
        with wave.open(str(args.output/'capture.wav'), 'wb') as w:
            w.setparams((2,2,RATE,0,'NONE','')); w.writeframes(pcm.read_bytes())
        pcm.unlink()
        info = summary(args.output/'capture.wav',60)
        record['capture'] = info
        if any(c['peak_dbfs'] is not None and c['peak_dbfs'] >= -3 for c in info['channels']):
            raise RuntimeError('Capture clipping/headroom refusal')
        record['state'] = 'captured_pending_owner_report_review'
    except BaseException as error:
        record['fault'] = str(error) or type(error).__name__
        raise
    finally:
        if remote_pgid is not None:
            cleanup = 'import os,signal; '
            cleanup += 'exec('+repr('try:\n os.killpg('+str(remote_pgid)+',signal.SIGTERM)\nexcept ProcessLookupError: pass')+')'
            try:
                subprocess.run(['ssh','gigpies-pi4','python3 -c '+shlex.quote(cleanup)],timeout=8,check=False)
            except subprocess.SubprocessError:
                record['remote_cleanup'] = 'unconfirmed; finite remote timeout remains active'
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try: child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill(); child.wait()
        for log in logs: log.close()
        record['ownership_wall_seconds'] = time.monotonic()-started
        checks=[]
        for command in (['sh','-c','fuser /dev/snd/pcm*'],['ssh','gigpies-pi4','fuser /dev/snd/pcm*']):
            try:
                check=subprocess.run(command,capture_output=True,timeout=8)
                checks.append(check.returncode==1 and not check.stdout.strip())
            except subprocess.SubprocessError:
                checks.append(False)
        record['devices_released'] = all(checks)
        (args.output/'session.json').write_text(json.dumps(record,indent=2)+'\n')
        signal.signal(signal.SIGTERM, previous_term)


def report(wav, directory, channel):
    """Delegate spectrum rendering to SoX; no transfer/calibration/EQ claims."""
    result = summary(wav)
    directory.mkdir(parents=True, exist_ok=False)
    (directory / 'levels.json').write_text(json.dumps(result, indent=2)+'\n')
    run(['sox', str(wav), '-n', 'remix', str(channel), 'spectrogram',
         '-x', '1200', '-y', '513', '-z', '90', '-t',
         'Uncalibrated microphone spectrum (dBFS; independent clock)',
         '-o', str(directory / 'spectrum.png')])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    p = sub.add_parser('prepare', help='Offline, bounded single-speaker signals')
    p.add_argument('directory', type=Path)
    p.add_argument('--channel', choices=['left', 'right'], required=True)
    p.add_argument('--level', type=float, default=-42)
    p = sub.add_parser('inspect', help='Offline PCM levels and channel identity')
    p.add_argument('wav', type=Path)
    p = sub.add_parser('report', help='Offline SoX spectrogram and PCM diagnostics')
    p.add_argument('wav', type=Path)
    p.add_argument('directory', type=Path)
    p.add_argument('--mic-channel', type=int, choices=[1, 2], required=True)
    p = sub.add_parser('capture', help='Explicit attended hardware action')
    p.add_argument('output', type=Path)
    p.add_argument('--capture-device', default='hw:E205U,0')
    p.add_argument('--playback-device', default='hw:AUDIO,0')
    p.add_argument('--channel', choices=['left', 'right'], default='left')
    p.add_argument('--play', type=Path)
    p.add_argument('--authorize-audio', action='store_true')
    p = sub.add_parser('live', help='Bounded production provider/Brain acquisition; prepared configs required')
    p.add_argument('output', type=Path)
    for name in ['provider', 'binary', 'ready', 'brain', 'control', 'local-binary']:
        p.add_argument('--'+name, type=Path, required=True)
    p.add_argument('--seconds', type=int, required=True)
    p.add_argument('--authorize-audio', action='store_true')
    args = parser.parse_args()
    if args.action == 'prepare':
        prepare(args.directory, args.channel, args.level)
    elif args.action == 'live':
        live(args)
    elif args.action == 'inspect':
        print(json.dumps(summary(args.wav), indent=2))
    elif args.action == 'report':
        report(args.wav, args.directory, args.mic_channel)
    else:
        capture(args)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
