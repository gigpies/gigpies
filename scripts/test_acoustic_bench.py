import array
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
import wave
import json
from types import SimpleNamespace

import acoustic_bench as bench


class BenchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / 'test.wav'

    def wav(self, values, rate=48000):
        data = array.array('h', values)
        if sys.byteorder != 'little':
            data.byteswap()
        with wave.open(str(self.path), 'wb') as w:
            w.setparams((2, 2, rate, 0, 'NONE', ''))
            w.writeframes(data.tobytes())

    def test_speaker_isolation_and_level(self):
        self.wav([100, 0] * 100)
        self.assertGreater(bench.validate_playback(self.path, 'left'), 0)
        with self.assertRaises(ValueError):
            bench.validate_playback(self.path, 'right')
        self.wav([2000, 0] * 100)
        with self.assertRaises(ValueError):
            bench.validate_playback(self.path, 'left')

    def test_format_and_silence_rejected_for_playback(self):
        self.wav([0, 0] * 100)
        with self.assertRaises(ValueError):
            bench.validate_playback(self.path, 'left')
        self.wav([100, 0], 44100)
        with self.assertRaises(ValueError):
            bench.summary(self.path)

    def test_channel_identity_and_clip_diagnostics(self):
        self.wav([32767, 32767, -32768, -32768])
        report = bench.summary(self.path)
        self.assertTrue(report['channels_bit_identical'])
        self.assertEqual(report['channels'][0]['full_scale_samples'], 2)
        self.assertFalse(report['absolute_latency_qualified'])

    def test_capture_requires_authorization_before_any_process(self):
        args = type('Args', (), {'authorize_audio': False})()
        with patch.object(subprocess, 'Popen') as popen:
            with self.assertRaises(ValueError):
                bench.capture(args)
            popen.assert_not_called()

    def test_position_id_matches_measurement_contract(self):
        self.assertEqual(bench.position_id('front-left'), 'front-left')
        for value in ('', 'two words', 'A'*129, 'é', 'p\n'):
            with self.assertRaises(bench.argparse.ArgumentTypeError):
                bench.position_id(value)

    def test_capture_failure_never_starts_playback(self):
        self.wav([100, 0] * 100)
        args = SimpleNamespace(authorize_audio=True, capture_device='hw:E205U,0',
                               playback_device='hw:AUDIO,0', play=self.path,
                               position_id='front-left', channel='left', output=Path(self.temp.name) / 'capture')
        child = Mock()
        child.poll.return_value = 1
        with patch.object(bench.subprocess, 'Popen', return_value=child), patch.object(bench, 'run') as run:
            with self.assertRaises(RuntimeError):
                bench.capture(args)
            run.assert_not_called()
        record = json.loads((args.output / 'session.json').read_text())
        self.assertEqual(record['state'], 'incomplete')
        self.assertEqual(record['position_id'], 'front-left')

    def test_playback_failure_reaps_capture(self):
        self.wav([100, 0] * 100)
        args = SimpleNamespace(authorize_audio=True, capture_device='hw:E205U,0',
                               playback_device='hw:AUDIO,0', play=self.path,
                               channel='left', output=Path(self.temp.name) / 'capture')
        child = Mock()
        child.poll.return_value = None
        def started(*unused, **kwargs):
            (args.output / 'capture.pcm').write_bytes(bytes(48000))
            return child
        with patch.object(bench.subprocess, 'Popen', side_effect=started), patch.object(
                bench, 'run', side_effect=subprocess.TimeoutExpired('ssh', 30)):
            with self.assertRaises(subprocess.TimeoutExpired):
                bench.capture(args)
        child.terminate.assert_called_once()
        child.wait.assert_called_once()
        self.assertEqual(json.loads((args.output / 'session.json').read_text())['state'], 'incomplete')

    def test_live_requires_authorization_and_free_devices(self):
        with patch.object(bench.subprocess, 'Popen') as popen:
            with self.assertRaises(ValueError):
                bench.live(SimpleNamespace(authorize_audio=False))
            args=SimpleNamespace(authorize_audio=True, seconds=18,
                provider=Path('/private/provider.json'),binary=Path('/private/runner'),
                ready=Path('/private/ready'),output=Path(self.temp.name)/'live')
            with patch.object(bench.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout=b'123')):
                with self.assertRaisesRegex(RuntimeError,'owner active'):
                    bench.live(args)
            popen.assert_not_called()
            self.assertFalse(args.output.exists())

    def test_generation_is_offline_and_bounded(self):
        # Real SoX smoke is an explicit preparation check, not a suite dependency.
        with patch.object(bench, 'run') as run, patch.object(bench, 'validate_playback'):
            bench.prepare(Path(self.temp.name) / 'signals', 'right', -42)
        self.assertEqual(run.call_count, 4)
        for call in run.call_args_list:
            self.assertEqual(call.args[0][0], 'sox')
            self.assertNotIn('aplay', call.args[0])
        with self.assertRaises(ValueError):
            bench.prepare(Path(self.temp.name) / 'bad', 'left', float('nan'))


if __name__ == '__main__':
    unittest.main()
