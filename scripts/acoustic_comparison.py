"""Operator report for production independent-clock acoustic EQ verification runs."""
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess


def read(path):
    return json.loads(path.read_text())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def evidence(run, session, proposal, side):
    request_file = run / 'eq-request.json'
    request = read(request_file)
    provider = read(run / 'provider.json')
    route = provider['acoustic_bench']
    state = provider['final_structure']
    require(session['state'] in ('reviewed_capture', 'captured_pending_owner_report_review') and session['devices_released'] is True
            and session['common_clock'] is False and session['microphone_calibration'] is None,
            'expected reviewed independent-clock uncalibrated capture')
    require(provider['fault'] is None and provider['source_report']['fault'] is None
            and provider['control_faults'] == provider['source_report']['acoustic_loss_packets']
            == provider['source_report']['egress_drops'] == 0
            and not any(provider['media_queue_drops'].values()), 'provider fault or packet loss')
    for name in ('provider', 'brain', 'control'):
        require(digest(run / (name+'.json')) == session.get(name+'_sha256'),
                f'unknown or mismatched {name} report identity')
    brain = read(run / 'brain.json')
    control = read(run / 'control.json')
    require(brain['fault'] is None and brain['incomplete_blocks'] == brain['stale_groups'] == 0
            and control['fault'] is None, 'Brain or control fault/incomplete media')
    capture = run / 'capture/capture.wav'
    require(Path(request['capture']).resolve() == capture.resolve()
            and digest(capture) == session['capture']['sha256'], 'capture identity mismatch')
    require(request['channel'] + 1 == session['microphone_channel'], 'microphone channel mismatch')
    gain = session.get('capture_gain_raw')
    require(isinstance(gain, list) and len(gain) == 2
            and all(type(x) is int and 0 <= x <= 34 for x in gain), 'unknown stereo capture gain')
    stimulus = session.get('excitation_sha256')
    require(stimulus and digest(Path(request['reference'])) == stimulus,
            'unknown or mismatched excitation digest')
    expected = proposal['basis_configuration' if side == 'baseline' else 'candidate_configuration']
    require(json.loads(state['pa_configuration_json']) == request['configuration'] == expected
            and request['configuration_revision'] == state['revision'],
            'PA readback differs from request or intended configuration')
    require(request['output_index'] == proposal['output_index']
            and request['target'] == proposal['target'], 'proposal output or target mismatch')
    require(route['mapping_confirmation'].strip() and route['excitation'], 'missing speaker mapping or excitation')
    basis = {key: route[key] for key in ('device', 'channel', 'mapping_confirmation',
                                         'expected_usb_id', 'pilot_bench', 'probe')}
    basis['fx_buses'] = [(bus['id'], json.loads(bus['configuration_json'])) for bus in control['applied']['buses']]
    basis.update(stimulus=stimulus, gain=gain, mic=request['channel'], basis=request['basis'],
                 reference_channel=request['reference_channel'], reference_window=request['reference_window'],
                 repeat_lengths=[b-a for a, b in request['repeats']])
    return request_file, basis


def analyze(owner, request):
    proc = subprocess.run([owner, '--acoustic-eq', str(request)], capture_output=True,
                          text=True, timeout=120)
    if proc.returncode:
        raise ValueError('SHR PA refused measurement: ' + (proc.stderr or proc.stdout).strip())
    result = json.loads(proc.stdout)
    require(result['contract'] == 'SHR-PA-INDEPENDENT-CLOCK-EQ' and result['version'] == 1,
            'unsupported owner result')
    return result


def measured(before, after):
    # Anchor both observations to the owner's baseline median and declared curve.
    rows = []
    corrected = {v['hz']: v for v in after['observations'] if v['usable']}
    for observation in before['observations']:
        hz = observation['hz']
        if not observation['usable'] or hz not in corrected:
            continue
        shape = {'flat': 0, 'bass_shaped': min(3, max(0, 3 * math.log2(200 / hz))),
                 'damped_high_frequency': max(-6, min(0, -1.5 * math.log2(hz / 2000)))}[before['target']]
        target = before['target_offset_db'] + shape
        a, b = observation['response_db'], corrected[hz]['response_db']
        rows.append(dict(hz=hz, baseline_db=a, corrected_db=b, change_db=b-a, target_db=target,
                         error_increase_db=abs(b-target)-abs(a-target)))
    require(len(rows) >= 12 and rows[-1]['hz'] >= 8 * rows[0]['hz'],
            'insufficient common reliable bands (12 bands spanning three octaves required)')
    def rms(key):
        return math.sqrt(sum((r[key]-r['target_db'])**2 for r in rows)/len(rows))
    a, b = rms('baseline_db'), rms('corrected_db')
    worst = max(r['error_increase_db'] for r in rows)
    if worst > 1 or b > a:
        outcome = 'regressed'
    elif a-b >= 0.25:
        outcome = 'improved'
    else:
        outcome = 'below_improvement_threshold'
    return dict(outcome=outcome, baseline_rms_error_db=a, corrected_rms_error_db=b,
                rms_improvement_db=a-b, worst_band_error_increase_db=worst, bands=rows)


def compare(args):
    proposal = read(args.proposal)
    require(proposal['contract'] == 'SHR-PA-INDEPENDENT-CLOCK-EQ' and proposal['version'] == 1
            and proposal['status'] == 'proposed'
            and proposal['basis_configuration'] != proposal['candidate_configuration'],
            'expected a reviewed SHR PA proposal with a configuration change')
    positions = {}
    invalid = []
    results = {}
    errors = (KeyError, ValueError, TypeError, AttributeError, OSError, subprocess.TimeoutExpired)
    for side in ('baseline', 'corrected'):
        for run in getattr(args, side):
            try:
                session = read(run / 'capture/session.json')
                position = session.get('position_id')
                require(isinstance(position, str) and re.fullmatch(r'[!-~]{1,128}', position),
                        'unknown position_id (unlabelled or invalid capture)')
                positions.setdefault(position, {}).setdefault(side, []).append((run, session))
            except errors as error:
                invalid.append(dict(run=str(run), side=side, reason=str(error)))
    for position, pair in sorted(positions.items()):
        try:
            requests = []
            bases = []
            for side in ('baseline', 'corrected'):
                runs = pair.get(side, [])
                require(len(runs) == 1, f'expected one {side} run, found {len(runs)}')
                request, basis = evidence(*runs[0], proposal, side)
                requests.append(request)
                bases.append(basis)
            for key in bases[0]:
                require(bases[0][key] == bases[1][key], f'mismatched {key}')
            results[position] = measured(*(analyze(args.owner, request) for request in requests))
        except errors as error:
            invalid.append(dict(position_id=position, reason=str(error)))
    return dict(common_clock=False, microphone_calibration=None, units='relative magnitude dB',
                positions=results, invalid=invalid)
