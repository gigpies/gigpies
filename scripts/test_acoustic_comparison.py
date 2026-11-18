import array
import copy
import json
import math
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest
import wave

from acoustic_comparison import digest, measured

SCRIPT = Path(__file__).resolve().with_name('acoustic_bench.py')
PA = SCRIPT.parents[2] / 'shr-pa'
OWNER = PA / 'target/release/shr-pa-measure'


def write(path, obj):
    path.write_text(json.dumps(obj))


def stereo(path, samples):
    pcm = array.array('h', (value for sample in samples for value in (sample, sample)))
    if sys.byteorder == 'big':
        pcm.byteswap()
    with wave.open(str(path), 'wb') as stream:
        stream.setparams((2, 2, 48000, 0, 'NONE', ''))
        stream.writeframes(pcm.tobytes())


class ComparisonTests(unittest.TestCase):
    def test_fixed_target_preserves_wideband_drop(self):
        bands = [dict(hz=125 * 2**(i/3), response_db=-7, usable=True) for i in range(19)]
        before = dict(target='flat', target_offset_db=-7, observations=bands)
        after = dict(target_offset_db=-9, observations=[dict(b, response_db=-9) for b in bands])
        result = measured(before, after)
        self.assertEqual(result['outcome'], 'regressed')
        self.assertEqual(result['corrected_rms_error_db'], 2)
        for band in after['observations'][8:]:
            band['usable'] = False
        with self.assertRaisesRegex(ValueError, 'insufficient common'):
            measured(before, after)

    @unittest.skipUnless(OWNER.exists(), 'requires built sibling shr-pa-measure')
    def test_operator_command_real_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = PA / 'tests/fixtures/measurement/v1/candidate.json'
            config = json.loads(json.loads(fixture.read_text())['configuration_json'])
            candidate = copy.deepcopy(config)
            candidate['outputs'][0]['processing']['eq'][0]['db'] = -2
            proposal = dict(contract='SHR-PA-INDEPENDENT-CLOCK-EQ', version=1, status='proposed',
                            target='flat', output_index=0, basis_configuration=config,
                            candidate_configuration=candidate)
            write(root/'proposal.json', proposal)
            rng = random.Random(42)
            flat = [rng.randint(-1800, 1800) for _ in range(48000)]
            colored = []
            # Add a narrow resonant component to the same deterministic stimulus.
            omega = 2 * math.pi * 1000 / 48000
            alpha = math.sin(omega) / 4
            x1 = x2 = y1 = y2 = 0
            for sample in flat:
                band = (alpha*(sample-x2) + 2*math.cos(omega)*y1 - (1-alpha)*y2) / (1+alpha)
                colored.append(round(sample + band))
                x2, x1, y2, y1 = x1, sample, y1, band
            reference = root/'stimulus.wav'
            stereo(reference, flat)

            def run(name, position, samples, corrected=False, gain=20):
                path = root/name
                folder = path/'capture'
                folder.mkdir(parents=True)
                capture = folder/'capture.wav'
                stereo(capture, [0]*48000 + samples*3)
                session = dict(state='reviewed_capture', common_clock=False, devices_released=True,
                               microphone_calibration=None, microphone_channel=1,
                               excitation_sha256=digest(reference), capture_gain_raw=[gain, gain],
                               capture=dict(sha256=digest(capture)))
                if position:
                    session['position_id'] = position
                graph = candidate if corrected else config
                route = dict(channel=1, device='fixture', expected_usb_id='fixture',
                             mapping_confirmation='right', excitation=str(reference),
                             probe={'repeats': 3}, pilot_bench=True)
                write(path/'provider.json', dict(acoustic_bench=route, fault=None, control_faults=0,
                      source_report=dict(fault=None, acoustic_loss_packets=0, egress_drops=0),
                      media_queue_drops={'media_in': 0}, final_structure=dict(revision='3',
                      pa_configuration_json=json.dumps(graph))))
                write(path/'eq-request.json', dict(capture=str(capture), channel=0,
                      repeats=[[48000*i, 48000*(i+1)] for i in (1, 2, 3)], noise=[0, 48000],
                      reference=str(reference), reference_channel=0, reference_window=[0, 48000],
                      basis='fixed gain and amp', configuration=graph, configuration_revision='3',
                      output_index=0, target='flat'))
                write(path/'brain.json', dict(fault=None, incomplete_blocks=0, stale_groups=0))
                write(path/'control.json', dict(fault=None, applied=dict(buses=[])))
                for report in ('brain', 'provider', 'control'):
                    session[report+'_sha256'] = digest(path/(report+'.json'))
                write(folder/'session.json', session)
                return path

            front = run('front-before', 'front', colored)
            rear = run('rear-before', 'rear', flat)
            front_after = run('front-after', 'front', flat, True)
            rear_after = run('rear-after', 'rear', colored, True)
            command = [sys.executable, SCRIPT, 'compare', '--owner', OWNER,
                       '--proposal', root/'proposal.json']
            def invoke(before, after, expected):
                proc = subprocess.run(command + ['--baseline', *before, '--corrected', *after],
                                      capture_output=True, text=True)
                self.assertEqual(proc.returncode, expected, proc.stdout + proc.stderr)
                return json.loads(proc.stdout)
            report = invoke([front, rear], [rear_after, front_after], 0)
            self.assertEqual(report['positions']['front']['outcome'], 'improved')
            self.assertEqual(report['positions']['rear']['outcome'], 'regressed')
            legacy = run('legacy', None, flat)
            mismatch = run('mismatched', 'front', flat, True, 19)
            report = invoke([front, rear, legacy], [mismatch], 2)
            reasons = [i['reason'] for i in report['invalid']]
            self.assertIn('mismatched gain', reasons)
            self.assertIn('expected one corrected run, found 0', reasons)
            self.assertTrue(any('unknown position_id' in reason for reason in reasons))
            report = invoke([front, rear], [front_after, rear_after, rear_after], 2)
            self.assertIn('front', report['positions'])
            self.assertEqual(report['invalid'][0]['position_id'], 'rear')
