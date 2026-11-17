#!/usr/bin/env python3
"""Bounded offline vocal decisions from frozen production ensemble observations.

Policy and capture schema: docs/guides/BALANCE_PASS.md. Never opens audio devices.
"""
import argparse
import copy
import hashlib
import itertools
import json
import math
import subprocess
import tempfile
from pathlib import Path

from spectral_balance import array_items, db, target_curve, write_json

POLICY = dict(version='vocal-contribution-v1', ratio_db=[-4., -2.],
              ranges_hz=[[120., 5000.], [1600., 5000.]], combined_ceiling_db=1.,
              max_delta_db=6., step_db=.5, fader_bounds_db=[-60., 12.],
              minimum_windows=5, minimum_seconds=.5, raw_floor_dbfs=-70.,
              minimum_confidence=.8, maximum_spread_db=12., minimum_improvement=.05, held_out_tolerance=.05)
# Eligibility only: isolation is an explicit capture attestation, never inferred.
ACTIVITY = dict(version='vocal-eligibility-v1', raw_floor_dbfs=-70.,
                minimum_windows=5, minimum_seconds=.5, minimum_confidence=.8)
CLASSIFIER = dict(version='capture-spectral-contrast-v1', radius=.10, separation=.30,
                  minimum_windows=5, minimum_seconds=.5, maximum_examples=4096)
STATES = dict(CLASSIFIER, version='capture-spectral-states-v1', maximum_states_per_class=8)
MIXTURE = dict(version='bleed-power-mixture-exclusion-v1', numerical_margin=1e-12)
CONTRAST_MODES = ('spectral_contrast', 'spectral_states')
CONTEXT = dict(version='vocal-lineup-protection-v1', training_tolerance=1e-9,
               held_out_tolerance=0.05, replay_tolerance=1e-6)
STATE_CONTEXT = dict(version='vocal-state-protection-v1', training_tolerance=1e-9,
                     held_out_tolerance=0.05, replay_tolerance=1e-6)
SOLO = dict(version='solo-anchored-vocal-v1', maximum_singers=8, maximum_passes=12,
            reconstruction_tolerance_db=.01, anchor='sole eligible singer in own role group')
KINDS = {'inherited', 'preset', 'measured_calibration', 'operator_override', 'solver_selected'}
NAMES = ('lead', 'backing', 'accompaniment')
STAGE = 'routed_before_excitation_fx_master'
FX_STAGE = 'excited_direct_plus_linear_returns_before_master'
ATTRIBUTION = dict(method='source-local-excitation-linear-returns-v1',
                   absolute_tolerance=1e-7, relative_tolerance=1e-5, tail_floor=1e-5)



def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def percentile(rows, fraction):
    ordered = sorted(rows)
    threshold = sum(w for _, w in ordered) * fraction
    count = 0
    for value, weight in ordered:
        count += weight
        if count >= threshold:
            return value
    return ordered[-1][0]


def prepare(state, metadata, capture):
    """Validate bindings before interpreting any numerical evidence."""
    session = state['baseline']
    channels = session['channels']
    n = len(channels)
    if capture['frozen_id'] != state['frozen']['id']:
        raise ValueError('capture does not bind this frozen measurement')
    if metadata['stage_names'][4] not in (STAGE, FX_STAGE) or metadata['sample_rate'] != session['sample_rate']:
        raise ValueError('incompatible processing stage or clock')
    edges = metadata['band_edges_hz']
    if (len(edges) < 5 or not all(finite(x) for x in edges) or edges[0] != 0
            or any(a >= b for a, b in zip(edges, edges[1:]))):
        raise ValueError('invalid spectral edges')
    for bounds in POLICY['ranges_hz']:
        if any(x not in edges for x in bounds):
            raise ValueError('objective requires exact supported range boundaries')
    split = capture['training_end_frame']
    if type(split) is not int or not 0 < split < metadata['frames']:
        raise ValueError('explicit training/held-out boundary required')
    provenance = capture['provenance']
    if len(provenance) != n or any(x not in KINDS for x in provenance):
        raise ValueError('explicit control provenance required for every channel')
    groups = capture['groups']
    if set(groups) != set(NAMES):
        raise ValueError('exactly lead/backing/accompaniment partitions required')
    members = [c for name in NAMES for c in groups[name]['channels']]
    if any(type(c) is not int for c in members) or sorted(members) != list(range(n)):
        raise ValueError('groups must partition all channels exactly once')
    profiles = state['request'].get('observation_groups', [])
    measured = dict(metadata.get('observation_groups', []))
    indices = []
    locked = []
    for name in NAMES:
        group = groups[name]
        cs = group['channels']
        if not cs:
            raise ValueError('empty group')
        obs = group['observation']
        if len(cs) == 1:
            if obs != cs[0]:
                raise ValueError('singleton observation mismatch')
            indices.append(obs)
        else:
            matches = [p for p in profiles if p['name'] == obs]
            if len(matches) != 1 or sorted(matches[0]['channels']) != sorted(cs) or obs not in measured:
                raise ValueError('missing matching coherent profile observation')
            indices.append(measured[obs])
        locked.append(any(provenance[c] == 'operator_override' for c in cs))
        if name == 'accompaniment':
            continue
        performances = group.get('performances', [])
        if sorted(c for p in performances for c in p['channels']) != sorted(cs):
            raise ValueError('performance identities must partition vocal microphones')
        for p in performances:
            if not p['singer'] or p['capture'] not in ('direct', 'double', 'room', 'bleed', 'unknown'):
                raise ValueError('missing singer/capture identity')
            if not finite(p['confidence']) or not 0 <= p['confidence'] <= 1:
                raise ValueError('invalid capture confidence')
            activity = p.get('activity', {'mode': 'reviewed_spans'})
            if not isinstance(activity, dict) or activity.get('mode') not in ('reviewed_spans', 'isolated_voice', *CONTRAST_MODES):
                raise ValueError('unsupported activity evidence mode')
            if activity['mode'] == 'isolated_voice':
                if (activity.get('attestation') != 'voice_only_no_bleed'
                        or not isinstance(activity.get('review_basis'), str)
                        or not activity['review_basis'].strip() or p.get('spans')):
                    raise ValueError('isolated activity requires capture review basis, voice-only attestation and no spans')
            if activity['mode'] in CONTRAST_MODES:
                validate_contrast(activity, p, split, metadata['frames'])
            end = 0
            for a, b in p.get('spans', []):
                if type(a) is not int or type(b) is not int or not end <= a < b <= metadata['frames']:
                    raise ValueError('invalid/overlapping activity spans')
                end = b
    lead_singers = {p['singer'] for p in groups['lead']['performances']}
    if lead_singers & {p['singer'] for p in groups['backing']['performances']}:
        raise ValueError('same singer/doubles cannot straddle independently moved groups')
    for c in channels:
        if not finite(c['fader_db']) or not -60 <= c['fader_db'] <= 12:
            raise ValueError('baseline fader outside supported bounds')
    return indices, locked


def validate_contrast(activity, performance, split, end):
    states = activity['mode'] == 'spectral_states'
    assumption = 'stable_capture_spectral_states' if states else 'stable_capture_spectral_separation'
    cutoff = activity.get('calibration_end_frame')
    if (type(cutoff) is not int or not 0 < cutoff < split or performance.get('spans')
            or activity.get('assumption') != assumption
            or not isinstance(activity.get('review_basis'), str)
            or not activity['review_basis'].strip()):
        raise ValueError('spectral contrast requires stable capture review, calibration prefix and no activity spans')
    previous = 0
    examples = activity.get('examples', [])
    if not isinstance(examples, list) or not examples:
        raise ValueError('reviewed direct/bleed examples required')
    calibration_states = {label: set() for label in ('direct', 'bleed')}
    for example in examples:
        a, b = example['span']
        if (type(a) is not int or type(b) is not int or not previous <= a < b <= end
                or not (b <= cutoff or a >= split)
                or example['label'] not in ('direct', 'bleed')):
            raise ValueError('ordered disjoint calibration/held-out direct/bleed examples required')
        if states:
            state = example.get('state')
            if not isinstance(state, str) or not state.strip() or len(state) > 64:
                raise ValueError('explicit bounded reviewed spectral state required')
            known = calibration_states[example['label']]
            if b <= cutoff:
                known.add(state)
                if len(known) > STATES['maximum_states_per_class']:
                    raise ValueError('spectral state budget exceeded')
            elif state not in known:
                raise ValueError('held-out spectral state absent from calibration; no discovery/refit')
        elif 'state' in example:
            raise ValueError('state annotations require spectral_states mode')
        previous = b
    if states and any(not known for known in calibration_states.values()):
        raise ValueError('calibration states required for both direct and bleed')


class SpectralContrast:
    """Capture-specific spectral discrimination; no universal voice model.

    Freeze a compact positive/negative template on a calibration prefix. Neither
    decision-training nor held-out frames update it. Unknown spectra abstain.
    """
    def __init__(self, activity, rate, bands):
        self.activity, self.rate, self.bands = activity, rate, bands
        self.states = activity['mode'] == 'spectral_states'
        self.labels = ({self.key(e): e['label'] for e in activity['examples']
                        if e['span'][1] <= activity['calibration_end_frame']}
                       if self.states else {k: k for k in ('direct', 'bleed')})
        self.samples = {k: [] for k in sorted(self.labels)}
        self.class_counts = dict(direct=0, bleed=0)
        self.record = dict(policy=copy.deepcopy(STATES if self.states else CLASSIFIER), templates={}, reasons=[],
                           calibration={}, held_out={k: dict(windows=0, frames=0, errors=0)
                                                    for k in self.samples})
        if self.states:
            self.record['state_classes'] = dict(sorted(self.labels.items()))
        self.frozen = False
        self.last_end = 0
        self.direct_support = []

    def key(self, example):
        return (json.dumps([example['label'], example['state']], ensure_ascii=True)
                if self.states else example['label'])

    def feature(self, row):
        raw = row['stages'][0]
        power = raw['band_power']
        if (not finite(raw['mean_power']) or raw['mean_power'] < 0
                or len(power) != self.bands
                or any(not finite(x) or x < 0 for x in power)):
            raise ValueError('invalid raw spectral contrast evidence')
        total = sum(power)
        if not finite(total):
            raise ValueError('nonfinite spectral contrast sum')
        if db(raw['mean_power']) <= ACTIVITY['raw_floor_dbfs'] or total <= 1e-18:
            return None
        return [math.sqrt(x/total) for x in power]

    @staticmethod
    def distance(a, b):
        return math.sqrt(sum((x-y)**2 for x, y in zip(a, b))/2)

    def freeze(self):
        if self.frozen:
            return
        self.frozen = True
        for label, samples in self.samples.items():
            frames = sum(n for _, n in samples)
            self.record['calibration'][label] = dict(windows=len(samples), frames=frames)
            if len(samples) < CLASSIFIER['minimum_windows'] or frames/self.rate < CLASSIFIER['minimum_seconds']:
                self.record['reasons'].append(label+': insufficient calibration')
                continue
            center = [sum(x[b]*n for x, n in samples)/frames for b in range(self.bands)]
            self.record['templates'][label] = center
            if any(self.distance(x, center) > CLASSIFIER['radius'] for x, _ in samples):
                self.record['reasons'].append(label+': calibration spectral variability')
        centers = self.record['templates']
        if any(self.distance(a, b) < CLASSIFIER['separation']
               for ka, a in centers.items() for kb, b in centers.items()
               if self.labels[ka] == 'direct' and self.labels[kb] == 'bleed'):
            self.record['reasons'].append('ambiguous direct/bleed calibration spectra')
        if self.states:
            self.exclude_bleed_mixtures()
        self.samples.clear()

    def exclude_bleed_mixtures(self):
        """Certify separation from an enclosure, never recognize a mixture.

        A radius-r ball in our metric gives coordinate bounds c +/- sqrt(2)*r.
        Squaring nonnegative bounds encloses each normalized power spectrum.
        Every convex power mixture remains between the coordinate extrema;
        taking square roots gives the same feature bounds. The box contains
        more than the realizable mixtures, so failure is uncertainty, not proof
        of bleed. No covariance or coherent-interference claim follows.
        """
        centers = self.record['templates']
        bleed = [c for k, c in centers.items() if self.labels[k] == 'bleed']
        if len(bleed) < 2:
            return
        radius = CLASSIFIER['radius']
        padding = math.sqrt(2)*radius
        lower = [max(0., min(c[b] for c in bleed)-padding) for b in range(self.bands)]
        upper = [min(1., max(c[b] for c in bleed)+padding) for b in range(self.bands)]
        distances = {k: self.distance(c, [min(hi, max(lo, x))
                     for x, lo, hi in zip(c, lower, upper)])
                     for k, c in centers.items() if self.labels[k] == 'direct'}
        self.record['bleed_mixture_exclusion'] = dict(policy=copy.deepcopy(MIXTURE),
            feature_lower=lower, feature_upper=upper, direct_distance_lower_bounds=distances)
        for key, distance in distances.items():
            if distance <= radius+MIXTURE['numerical_margin']:
                self.record['reasons'].append(key+': direct support not separated from bleed-power mixture enclosure')

    def classify(self, feature):
        self.direct_support = []
        if feature is None:
            return 'below_floor'
        if self.record['reasons']:
            return 'unsupported_contrast'
        centers = self.record['templates']
        support = [key for key, center in centers.items()
                   if self.distance(feature, center) <= CLASSIFIER['radius']]
        matches = {self.labels[key] for key in support}
        if self.states and matches == {'direct'}:
            self.direct_support = sorted(support)
        return 'contrast_'+next(iter(matches)) if len(matches) == 1 else 'ambiguous_spectrum'

    def observe(self, row):
        self.direct_support = []
        start, frames = row['start'], row['frames']
        if (type(start) is not int or type(frames) is not int or frames <= 0
                or start < self.last_end):
            raise ValueError('invalid spectral contrast timeline')
        self.last_end = start+frames
        feature = self.feature(row)
        label = next((self.key(e) for e in self.activity['examples']
                      if e['span'][0] <= start and start+frames <= e['span'][1]), None)
        cutoff = self.activity['calibration_end_frame']
        if start < cutoff:
            if start+frames <= cutoff and label:
                if feature is None:
                    reason = label+': unusable reviewed calibration example'
                    if reason not in self.record['reasons']:
                        self.record['reasons'].append(reason)
                else:
                    samples = self.samples[label]
                    kind = self.labels[label]
                    if self.class_counts[kind] >= CLASSIFIER['maximum_examples']:
                        raise ValueError('spectral contrast calibration budget exceeded')
                    self.class_counts[kind] += 1
                    samples.append((feature, frames))
            return 'contrast_calibration'
        self.freeze()
        reason = self.classify(feature)
        if label:
            coverage = self.record['held_out'][label]
            coverage['windows'] += 1
            coverage['frames'] += frames
            center = self.record['templates'].get(label)
            coverage['errors'] += (reason != 'contrast_'+self.labels[label]
                or center is None or feature is None
                or self.distance(feature, center) > CLASSIFIER['radius'])
        return reason

    def finish(self):
        self.freeze()
        for label, coverage in self.record['held_out'].items():
            if (coverage['windows'] < CLASSIFIER['minimum_windows']
                    or coverage['frames']/self.rate < CLASSIFIER['minimum_seconds']
                    or coverage['errors']):
                self.record['reasons'].append(label+': held-out classification incomplete or failed; no refit')
        return self.record


def eligibility(performance, raw, start, frames):
    # This floor excludes unusable energy; it does not classify the sound.
    if not finite(raw) or raw < 0:
        raise ValueError('invalid raw activity evidence')
    if db(raw) <= ACTIVITY['raw_floor_dbfs']:
        return 'below_floor'
    if performance['capture'] not in ('direct', 'double'):
        return 'unsupported_capture'
    if performance['confidence'] < ACTIVITY['minimum_confidence']:
        return 'low_review_confidence'
    if performance.get('activity', {}).get('mode') == 'isolated_voice':
        return 'isolated_energy'
    if any(a <= start and start+frames <= z for a, z in performance.get('spans', [])):
        return 'reviewed_span'
    return 'unreviewed_energy'


def metrics(deltas, subset):
    factors = [10**(d/10) for d in deltas]
    ratios = [[[] for _ in POLICY['ranges_hz']] for _ in range(2)]
    combined = [[] for _ in POLICY['ranges_hz']]
    loss, weight = 0., 0.
    group_loss, group_weight = [0., 0.], [0., 0.]
    contexts = {}
    state_contexts = {}
    for e in subset:
        for i in range(2):
            if e['active'][i]:
                # Protect each support separately, not a Cartesian combination
                # or a nearest-state assignment. Overlapping states all count.
                state_keys = []
                for channel, states in e['state_support'][i]:
                    for state in states:
                        key = json.dumps([e['lineup'], i, channel, state], separators=(',', ':'))
                        state_contexts.setdefault(key, dict(lineup=e['lineup'], group=i,
                            channel=channel, state=state, loss_sum=0., weight=0))
                        state_keys.append(key)
                for b in range(2):
                    ratio = db(e['power'][i][b]*factors[i])-db(e['denominator'][b])
                    ratios[i][b].append((ratio, e['frames']))
                    error = max(POLICY['ratio_db'][0]-ratio, 0., ratio-POLICY['ratio_db'][1])
                    loss += e['frames']*error*error
                    weight += e['frames']
                    group_loss[i] += e['frames']*error*error
                    group_weight[i] += e['frames']
                    # The complete eligible lineup distinguishes solo/overlap and
                    # alternate captures without inventing independent powers.
                    key = json.dumps(e['lineup'], separators=(',', ':'))
                    context = contexts.setdefault(key, dict(lineup=e['lineup'],
                        loss_sum=[0., 0.], weights=[0, 0]))
                    context['loss_sum'][i] += e['frames']*error*error
                    context['weights'][i] += e['frames']
                    for key in state_keys:
                        row = state_contexts[key]
                        row['loss_sum'] += e['frames']*error*error
                        row['weight'] += e['frames']
        if all(e['active']):
            for b in range(2):
                power = sum(e['power'][i][b]*factors[i] for i in range(2)) + e['cross'][b]*math.sqrt(factors[0]*factors[1])
                combined[b].append(db(max(0., power))-db(e['denominator'][b]))
    context_losses = {key: dict(lineup=row['lineup'],
        group_loss=[a/b if b else None for a, b in zip(row['loss_sum'], row['weights'])],
        frames=[w//len(POLICY['ranges_hz']) for w in row['weights']])
        for key, row in sorted(contexts.items())}
    state_losses = {key: dict(lineup=row['lineup'], group=row['group'], channel=row['channel'],
        state=row['state'], loss=row['loss_sum']/row['weight'],
        frames=row['weight']//len(POLICY['ranges_hz'])) for key, row in sorted(state_contexts.items())}
    return dict(state_losses=state_losses, context_losses=context_losses, loss=loss/weight, group_loss=[a/b for a, b in zip(group_loss, group_weight)], ratios_db=[[
        dict(p10=percentile(r, .1), p50=percentile(r, .5), p90=percentile(r, .9)) for r in group] for group in ratios],
        combined_windows_db=combined,
        combined_max_db=[max(r) if r else None for r in combined])

def context_regressions(after, before, tolerance):
    return [key for key, old in before['context_losses'].items()
            if key not in after['context_losses'] or any(
                a is None or a > b+tolerance
                for a, b in zip(after['context_losses'][key]['group_loss'], old['group_loss'])
                if b is not None)]


def state_regressions(after, before, tolerance):
    return [key for key, old in before['state_losses'].items()
            if key not in after['state_losses'] or after['state_losses'][key]['loss'] > old['loss']+tolerance]


def calculate(state, metadata, windows, capture, measure_only=False):
    mode = capture.get('decision_mode', 'common')
    if mode == 'solo_anchored':
        return calculate_singers(state, metadata, windows, capture, measure_only)
    if mode != 'common':
        raise ValueError('unsupported vocal decision mode')
    indices, locked = prepare(state, metadata, capture)
    baseline = state['baseline']
    result = dict(policy=copy.deepcopy(POLICY), frozen_id=state['frozen']['id'],
                  evidence=copy.deepcopy(capture), source_identities=state['frozen'].get('inputs', []),
                  stage=metadata['stage_names'][4], attribution=copy.deepcopy(metadata.get('attribution')),
                  attribution_policy=copy.deepcopy(ATTRIBUTION) if state['request'].get('vocal_fx_attribution') else None, status='refused', reasons=[], confidence=0.,
                  settings=copy.deepcopy(baseline), decisions=[], metrics_before=None,
                  metrics_predicted=None, metrics_resulting=None,
                  limitations=['Pre-master spectral proxy, not proven audibility.',
                               'No prediction through nonlinear master or unmeasured returns.'])
    result['context_policy'] = copy.deepcopy(CONTEXT)
    result['state_context_policy'] = copy.deepcopy(STATE_CONTEXT)
    result['activity_policy'] = copy.deepcopy(ACTIVITY)
    result['policy_sha256'] = hashlib.sha256(json.dumps(POLICY, sort_keys=True).encode()).hexdigest()
    def finish(deltas=(0., 0.)):
        for i, name in enumerate(NAMES[:2]):
            for c in capture['groups'][name]['channels']:
                old = baseline['channels'][c]['fader_db']
                result['settings']['channels'][c]['fader_db'] = old + deltas[i]
                result['decisions'].append(dict(channel=c, file=baseline['channels'][c]['file'],
                    group=name, provenance_before=capture['provenance'][c],
                    provenance_after='solver_selected' if deltas[i] else capture['provenance'][c],
                    before_db=old, proposed_db=old+deltas[i], delta_db=deltas[i],
                    locked=locked[i], constraints=['common coherent group move', 'bounded grid',
                                                   'combined contribution guard']))
        return result
    if baseline.get('effects') is not None or metadata['stage_names'][4] == FX_STAGE:
        attribution = metadata.get('attribution') or {}
        if (not state['request'].get('vocal_fx_attribution') or metadata['stage_names'][4] != FX_STAGE
                or any(attribution.get(k) != v for k, v in ATTRIBUTION.items())
                or attribution.get('checked_frames') != metadata['frames']
                or not finite(attribution.get('max_tolerance_fraction'))
                or not 0 <= attribution['max_tolerance_fraction'] <= 1
                or not finite(attribution.get('max_absolute_error'))
                or attribution['max_absolute_error'] < 0):
            result['reasons'].append('unsupported excitation/post-fader FX attribution: regenerate with vocal_fx_attribution')
            return finish()
        if (attribution.get('tail_complete') is not True or not finite(attribution.get('tail_peak'))
                or not 0 <= attribution['tail_peak'] <= ATTRIBUTION['tail_floor']):
            result['reasons'].append('incomplete FX tail: extend the frozen tail and recapture; no refit')
            return finish()
    edges = metadata['band_edges_hz']
    ranges = [[b for b, (lo, hi) in enumerate(zip(edges, edges[1:])) if lo >= a and hi <= z]
              for a, z in POLICY['ranges_hz']]
    evidence = []
    skipped = [0, 0]
    confidence = []
    activity_evidence = {c: dict(channel=c, counts={}, training=dict(windows=0, frames=0),
                                 held_out=dict(windows=0, frames=0))
                         for name in NAMES[:2] for c in capture['groups'][name]['channels']}
    result['activity_evidence'] = list(activity_evidence.values())
    classifiers = {c: SpectralContrast(p['activity'], metadata['sample_rate'], len(edges)-1)
                   for name in NAMES[:2] for p in capture['groups'][name]['performances']
                   if p.get('activity', {}).get('mode') in CONTRAST_MODES for c in p['channels']}
    classified = {}
    unsupported_routed = False
    pending = {}
    last_end = 0
    def consume(rows):
        nonlocal last_end, unsupported_routed
        start, frames = rows[0]['start'], rows[0]['frames']
        if (type(start) is not int or type(frames) is not int or frames <= 0
                or start < last_end or start+frames > metadata['frames']
                or any((r['start'], r['frames']) != (start, frames) for r in rows)):
            raise ValueError('inconsistent simultaneous timeline')
        last_end = start + frames
        powers = [r['stages'][4]['band_power'] for r in rows]
        rests = [r['competition']['rest_band_power'] for r in rows]
        if any(len(p) != len(edges)-1 or any(not finite(v) or v < 0 for v in p) for p in powers+rests):
            raise ValueError('unsupported/nonfinite spectral evidence')
        # The complement of each partition is the coherent sum of the other two.
        cross = [rests[2][b]-powers[0][b]-powers[1][b] for b in range(len(edges)-1)]
        for i, j, k in ((0, 1, 2), (0, 2, 1), (1, 2, 0)):
            for b in range(len(edges)-1):
                xy = rests[k][b]-powers[i][b]-powers[j][b]
                if abs(xy) > 2*math.sqrt(powers[i][b]*powers[j][b])+1e-9*max(1e-20, rests[k][b]):
                    raise ValueError('physically inconsistent coherent powers')
        try:
            pink = target_curve(edges, powers[2])
        except ValueError:
            return
        denominator = [max(sum(powers[2][b] for b in bs), sum(pink[b] or 0 for b in bs)) for bs in ranges]
        if min(denominator) <= 1e-18:
            return
        active = []
        lineup = []
        state_support = []
        for i, name in enumerate(NAMES[:2]):
            accepted = True
            any_active = False
            eligible_channels = []
            for p in capture['groups'][name]['performances']:
                for c in p['channels']:
                    r = pending.get(c)
                    if r is None or (r['start'], r['frames']) != (start, frames):
                        raise ValueError('missing simultaneous per-microphone evidence')
                    reason = eligibility(p, r['stages'][0]['mean_power'], start, frames)
                    if c in classifiers and reason == 'unreviewed_energy':
                        reason = classified[c]
                    counts = activity_evidence[c]['counts']
                    counts[reason] = counts.get(reason, 0) + 1
                    if reason != 'below_floor':
                        any_active = True
                        supported = reason in ('isolated_energy', 'reviewed_span', 'contrast_direct')
                        accepted &= supported
                        if supported:
                            confidence.append(p['confidence'])
                            eligible_channels.append(c)
            if accepted and any_active:
                split = capture['training_end_frame']
                label = 'training' if start+frames <= split else 'held_out' if start >= split else None
                if label:
                    for c in eligible_channels:
                        coverage = activity_evidence[c][label]
                        coverage['windows'] += 1
                        coverage['frames'] += frames
            if accepted and any_active and any(sum(powers[i][b] for b in bs) <= 1e-18 for bs in ranges):
                unsupported_routed = True
            lineup.append(sorted(eligible_channels) if accepted and any_active else [])
            state_support.append([[c, list(classifiers[c].direct_support)] for c in sorted(eligible_channels)
                if c in classifiers and classifiers[c].direct_support] if accepted and any_active else [])
            active.append(accepted and any_active)
            skipped[i] += not active[-1]
        if any(active):
            evidence.append(dict(start=start, frames=frames, active=active, lineup=lineup, state_support=state_support,
                power=[[sum(p[b] for b in bs) for bs in ranges] for p in powers[:2]],
                cross=[sum(cross[b] for b in bs) for bs in ranges], denominator=denominator))
    current = None
    for row in windows:
        key = (row['start'], row['frames'])
        if current is not None and key != current:
            if any(i not in pending for i in indices):
                raise ValueError('missing coherent group window')
            consume([pending[i] for i in indices])
            pending = {}
        current = key
        if row['channel'] in pending:
            raise ValueError('duplicate channel window')
        if row['channel'] in classifiers:
            classified[row['channel']] = classifiers[row['channel']].observe(row)
        pending[row['channel']] = row
    if pending:
        if any(i not in pending for i in indices):
            raise ValueError('missing coherent group window')
        consume([pending[i] for i in indices])
    for c, classifier in classifiers.items():
        activity_evidence[c]['spectral_contrast'] = classifier.finish()
        result['reasons'].extend(f'channel {c}: '+r for r in classifier.record['reasons'])
    result['accepted_windows'] = [sum(e['active'][i] for e in evidence) for i in range(2)]
    result['accepted_seconds'] = [sum(e['frames'] for e in evidence if e['active'][i])/metadata['sample_rate'] for i in range(2)]
    result['skipped_windows'] = skipped
    result['confidence'] = min(confidence, default=0.)
    # Retain compact sufficient evidence, not all production stage traces.
    result['windows'] = evidence
    for i, name in enumerate(NAMES[:2]):
        if result['accepted_windows'][i] < POLICY['minimum_windows'] or result['accepted_seconds'][i] < POLICY['minimum_seconds']:
            result['reasons'].append(name+': missing/sparse/bleed-dominated eligible performance evidence')
    for c, evidence_row in activity_evidence.items():
        for label in ('training', 'held_out'):
            coverage = evidence_row[label]
            if (coverage['windows'] < ACTIVITY['minimum_windows'] or
                    coverage['frames']/metadata['sample_rate'] < ACTIVITY['minimum_seconds']):
                result['reasons'].append(f'{label}: idle microphone or insufficient eligible evidence for channel {c}; '
                                         'review activity/bleed or provide a supported capture')
    if unsupported_routed:
        result['reasons'].append('active raw voice has missing routed contribution; inspect cancellation/mute/tone')
    split = capture['training_end_frame']
    training = [e for e in evidence if e['start']+e['frames'] <= split]
    held_out = [e for e in evidence if e['start'] >= split]
    for label, subset in [('training', training), ('held_out', held_out)]:
        overlap = [e for e in subset if all(e['active'])]
        if len(overlap) < POLICY['minimum_windows'] or sum(e['frames'] for e in overlap)/metadata['sample_rate'] < POLICY['minimum_seconds']:
            result['reasons'].append(label+': insufficient simultaneous lead/backing evidence for combined evaluation')
        for i, name in enumerate(NAMES[:2]):
            selected = [e for e in subset if e['active'][i]]
            if len(selected) < POLICY['minimum_windows'] or sum(e['frames'] for e in selected)/metadata['sample_rate'] < POLICY['minimum_seconds']:
                result['reasons'].append(label+': insufficient '+name+' evidence')
    if result['reasons']:
        return finish()
    before = metrics((0., 0.), training)
    result['metrics_before'] = before
    if measure_only:
        result['held_out_before'] = metrics((0., 0.), held_out)
        return result
    for i, name in enumerate(NAMES[:2]):
        rs = before['ratios_db'][i]
        if any(r['p90']-r['p10'] > POLICY['maximum_spread_db'] for r in rs):
            result['reasons'].append(name+': excessive dynamics require separate review')
        if min(r['p50'] for r in rs) < POLICY['ratio_db'][0] and max(r['p50'] for r in rs) > POLICY['ratio_db'][1]:
            result['reasons'].append(name+': contribution/articulation conflict requires tone review')
    if result['reasons']:
        return finish()
    choices = []
    for i, name in enumerate(NAMES[:2]):
        cs = capture['groups'][name]['channels']
        steps = round(POLICY['max_delta_db']/POLICY['step_db'])
        choices.append([d*POLICY['step_db'] for d in range(-steps, steps+1) if (not locked[i] or d == 0) and
                        all(POLICY['fader_bounds_db'][0] <= baseline['channels'][c]['fader_db']+d*POLICY['step_db'] <= POLICY['fader_bounds_db'][1] for c in cs)])
    best, best_metrics = (0., 0.), before
    for deltas in itertools.product(*choices):
        after = metrics(deltas, training)
        if context_regressions(after, before, CONTEXT['training_tolerance']):
            continue
        if state_regressions(after, before, STATE_CONTEXT['training_tolerance']):
            continue
        if any(a > b+1e-9 for a, b in zip(after['group_loss'], before['group_loss'])):
            continue
        if any(a > max(POLICY['combined_ceiling_db'], b)+1e-9 for new, old in zip(after['combined_windows_db'], before['combined_windows_db']) for a, b in zip(new, old)):
            continue
        if (after['loss'], sum(abs(d) for d in deltas), deltas) < (best_metrics['loss'], sum(abs(d) for d in best), best):
            best, best_metrics = deltas, after
    if before['loss']-best_metrics['loss'] < POLICY['minimum_improvement']:
        best, best_metrics = (0., 0.), before
    held_before = metrics((0., 0.), held_out)
    held_after = metrics(best, held_out)
    result['held_out_before'] = held_before
    result['held_out_proposed'] = held_after
    result['proposed_deltas_db'] = best
    result['held_out_context_vetoes'] = context_regressions(held_after, held_before, CONTEXT['held_out_tolerance'])
    result['held_out_state_vetoes'] = state_regressions(held_after, held_before, STATE_CONTEXT['held_out_tolerance'])
    if (result['held_out_state_vetoes'] or result['held_out_context_vetoes'] or any(a > b+POLICY['held_out_tolerance'] for a, b in zip(held_after['group_loss'], held_before['group_loss'])) or
            held_after['loss'] > held_before['loss']+POLICY['held_out_tolerance'] or
            any(a > max(POLICY['combined_ceiling_db'], b)+1e-9 for new, old in zip(held_after['combined_windows_db'], held_before['combined_windows_db']) for a, b in zip(new, old))):
        result['reasons'] = ['held-out veto: no refit permitted']
        result['metrics_predicted'] = best_metrics
        return finish()
    result['metrics_predicted'] = best_metrics
    result['status'] = 'candidate' if any(best) else 'no_change'
    result['objective_met'] = all(m['loss'] <= 1e-12 and all(x is None or x <= POLICY['combined_ceiling_db'] for x in m['combined_max_db']) for m in (best_metrics, held_after))
    if any(best):
        reason = 'bounded measured improvement'
    elif result['objective_met']:
        reason = 'measured lead/backing and combined contributions satisfy objective deadbands'
    elif any(locked[:2]):
        reason = 'operator override prevents a supported improvement; objective remains unresolved'
    else:
        reason = 'gain bounds, minimum improvement, lineup, spectral state or combined guard prevents a supported improvement'
    result['reasons'] = [reason]
    return finish(best)


def singer_units(capture):
    units = []
    for group, name in enumerate(NAMES[:2]):
        singers = {}
        for p in capture['groups'][name]['performances']:
            singers.setdefault(p['singer'], []).extend(p['channels'])
        for singer, channels in sorted(singers.items(), key=lambda item: min(item[1])):
            units.append(dict(singer=singer, group=group, channels=sorted(channels),
                locked=any(capture['provenance'][c] == 'operator_override' for c in channels)))
    if len(units) > SOLO['maximum_singers']:
        raise ValueError('independent singer search budget exceeded')
    return units


def singer_windows(windows, units, deltas):
    gains = [10**(d/20) for d in deltas]
    output = []
    for e in windows:
        powers = [[0., 0.], [0., 0.]]
        cross = [0., 0.]
        for i, unit in enumerate(units):
            for b in range(2):
                powers[unit['group']][b] += e['singer_power'][i][b]*gains[i]**2
        for i, j, xy in e['singer_cross']:
            for b in range(2):
                value = xy[b]*gains[i]*gains[j]
                if units[i]['group'] == units[j]['group']:
                    powers[units[i]['group']][b] += value
                else:
                    cross[b] += value
        # Signed terms are retained; tolerate only floating-point residue.
        if any(v < -1e-12 for p in powers for v in p):
            raise ValueError('negative coherent singer prediction')
        output.append(dict(e, power=[[max(0., v) for v in p] for p in powers], cross=cross))
    return output


def singer_metrics(deltas, windows, units):
    result = metrics((0., 0.), singer_windows(windows, units, deltas))
    anchors = []
    for i, unit in enumerate(units):
        rows = [e for e in windows if e['solo'][i]]
        ratios = [[(db(e['singer_power'][i][b])-db(e['denominator'][b])+deltas[i], e['frames'])
                   for e in rows] for b in range(2)]
        errors = [(max(POLICY['ratio_db'][0]-v, 0., v-POLICY['ratio_db'][1])**2, w)
                  for band in ratios for v, w in band]
        anchors.append(dict(windows=len(rows), frames=sum(e['frames'] for e in rows),
            loss=sum(v*w for v, w in errors)/sum(w for _, w in errors) if errors else None,
            ratios_db=[dict(p10=percentile(r, .1), p50=percentile(r, .5), p90=percentile(r, .9))
                       for r in ratios] if rows else []))
    result['anchors'] = anchors
    result['decision_loss'] = result['loss'] + sum(a['loss'] or 0. for a in anchors)/len(units)
    return result


def singer_regressions(after, before, tolerance):
    failures = []
    if any(a['loss'] is None or a['loss'] > b['loss']+tolerance
           for a, b in zip(after['anchors'], before['anchors']) if b['loss'] is not None):
        failures.append('solo anchor loss regression')
    if any(a > b+tolerance for a, b in zip(after['group_loss'], before['group_loss'])):
        failures.append('ensemble loss regression')
    if context_regressions(after, before, tolerance) or state_regressions(after, before, tolerance):
        failures.append('lineup or spectral state regression')
    if any(a > max(POLICY['combined_ceiling_db'], b)+1e-6
           for aa, bb in zip(after['combined_windows_db'], before['combined_windows_db'])
           for a, b in zip(aa, bb)):
        failures.append('combined ceiling regression')
    return failures


def calculate_singers(state, metadata, windows, capture, measure_only=False):
    # Reuse exactly the existing capture eligibility, classifier and FX contract.
    common = dict(capture, decision_mode='common')
    units = singer_units(capture)
    channels = sorted(c for u in units for c in u['channels'])
    required = list(itertools.combinations(channels, 2))
    if any(list(pair) not in state['request'].get('observation_pairs', []) for pair in required):
        raise ValueError('solo-anchored decisions require every routed vocal observation pair')
    edges = metadata['band_edges_hz']
    bands = [[b for b, (lo, hi) in enumerate(zip(edges, edges[1:])) if lo >= a and hi <= z]
             for a, z in POLICY['ranges_hz']]
    observed = {}
    def trace():
        for row in windows:
            if row['channel'] in channels:
                key = (row['start'], row['frames'])
                observed.setdefault(key, {})[row['channel']] = (
                    row['stages'][4]['band_power'], row.get('pair_sum_band_power', []))
            yield row
    result = calculate(state, metadata, trace(), common, measure_only=True)
    result.update(evidence=copy.deepcopy(capture), singer_policy=copy.deepcopy(SOLO), units=units)
    result['decisions'] = []
    zero = (0.,)*len(units)
    def finish(deltas=zero):
        result['settings'] = copy.deepcopy(state['baseline'])
        for i, unit in enumerate(units):
            for c in unit['channels']:
                old = state['baseline']['channels'][c]['fader_db']
                result['settings']['channels'][c]['fader_db'] = old+deltas[i]
                result['decisions'].append(dict(channel=c, file=state['baseline']['channels'][c]['file'],
                    group=NAMES[unit['group']], singer=unit['singer'], before_db=old,
                    proposed_db=old+deltas[i], delta_db=deltas[i], locked=unit['locked'],
                    provenance_before=capture['provenance'][c],
                    provenance_after='solver_selected' if deltas[i] else capture['provenance'][c],
                    constraints=['linked singer captures', 'solo anchor', 'coherent ensemble protection']))
        return result
    if result['reasons']:
        return finish()
    for e in result['windows']:
        raw = observed[(e['start'], e['frames'])]
        p = {c: raw[c][0] for c in channels}
        if any(len(power) != len(edges)-1 or any(not finite(v) or v < 0 for v in power)
               for power in p.values()):
            raise ValueError('invalid routed singer power')
        xy = {}
        for a, b in required:
            pairs = raw[a][1]
            matches = [power for other, power in pairs if other == b]
            if (len(matches) != 1 or len(matches[0]) != len(edges)-1
                    or any(not finite(v) or v < 0 for v in matches[0])):
                raise ValueError('missing/invalid simultaneous vocal pair measurement')
            values = [v-x-y for v, x, y in zip(matches[0], p[a], p[b])]
            if any(abs(v) > 2*math.sqrt(x*y)+1e-9*max(1e-20, x+y)
                   for v, x, y in zip(values, p[a], p[b])):
                raise ValueError('physically inconsistent vocal pair measurement')
            xy[a, b] = values
        def pair(a, b):
            return xy[min(a,b), max(a,b)]
        e['singer_power'] = [[sum(sum(p[c][b] for c in u['channels']) +
            sum(pair(a,c)[b] for a,c in itertools.combinations(u['channels'], 2)) for b in bs)
            for bs in bands] for u in units]
        e['singer_cross'] = [[i,j,[sum(pair(a,c)[b] for a in units[i]['channels']
            for c in units[j]['channels'] for b in bs) for bs in bands]]
            for i,j in itertools.combinations(range(len(units)), 2)]
        e['solo'] = [e['active'][u['group']] and bool(e['lineup'][u['group']])
            and set(e['lineup'][u['group']]).issubset(u['channels']) for u in units]
        reconstructed = singer_windows([e], units, zero)[0]
        if any(abs(db(a)-db(b)) > SOLO['reconstruction_tolerance_db']
               for aa, bb in zip(e['power'], reconstructed['power']) for a,b in zip(aa,bb)):
            raise ValueError('vocal pair reconstruction differs from measured coherent groups')
        for b, (a, actual) in enumerate(zip(e['cross'], reconstructed['cross'])):
            if abs(a-actual) > 1e-8*max(1e-20, sum(p[b] for p in e['power'])):
                raise ValueError('vocal pair reconstruction differs from combined cross power')
    observed.clear()
    split = capture['training_end_frame']
    subsets = [[e for e in result['windows'] if e['start']+e['frames'] <= split],
               [e for e in result['windows'] if e['start'] >= split]]
    before, held = [singer_metrics(zero, rows, units) for rows in subsets]
    result.update(metrics_before=before, held_out_before=held)
    for label, m in zip(('training', 'held_out'), (before, held)):
        for unit, anchor in zip(units, m['anchors']):
            if (anchor['windows'] < POLICY['minimum_windows'] or
                    anchor['frames']/metadata['sample_rate'] < POLICY['minimum_seconds']):
                result['reasons'].append(label+': insufficient solo anchor for '+unit['singer'])
    if result['reasons'] or measure_only:
        return finish()
    for unit, anchor in zip(units, before['anchors']):
        rs = anchor['ratios_db']
        if any(r['p90']-r['p10'] > POLICY['maximum_spread_db'] for r in rs):
            result['reasons'].append(unit['singer']+': solo dynamics require separate review')
        if min(r['p50'] for r in rs) < POLICY['ratio_db'][0] and max(r['p50'] for r in rs) > POLICY['ratio_db'][1]:
            result['reasons'].append(unit['singer']+': solo contribution/articulation conflict')
    if result['reasons']:
        return finish()
    step = POLICY['step_db']
    steps = round(POLICY['max_delta_db']/step)
    choices = [[d*step for d in range(-steps,steps+1) if (not u['locked'] or d == 0) and all(
        POLICY['fader_bounds_db'][0] <= state['baseline']['channels'][c]['fader_db']+d*step <= POLICY['fader_bounds_db'][1] for c in u['channels'])]
        for u in units]
    best, best_metrics = zero, before
    evaluations = 0
    # Finite deterministic coordinate + paired-step descent. Paired steps allow
    # redistribution under a ceiling without requiring an unsafe intermediate.
    for iteration in range(SOLO['maximum_passes']):
        candidates = set()
        for i in range(len(units)):
            for d in choices[i]:
                candidate = list(best); candidate[i] = d
                candidates.add(tuple(candidate))
        for i,j in itertools.combinations(range(len(units)), 2):
            for a,b in itertools.product((-step,step), repeat=2):
                candidate = list(best); candidate[i] += a; candidate[j] += b
                if candidate[i] in choices[i] and candidate[j] in choices[j]:
                    candidates.add(tuple(candidate))
        previous = best
        for deltas in sorted(candidates):
            m = singer_metrics(deltas, subsets[0], units)
            evaluations += 1
            if singer_regressions(m, before, 1e-9):
                continue
            if (m['decision_loss'], sum(map(abs,deltas)), deltas) < (best_metrics['decision_loss'], sum(map(abs,best)), best):
                best, best_metrics = deltas, m
        if best == previous:
            break
    result['search'] = dict(passes=iteration+1, evaluations=evaluations, global_optimum_claimed=False)
    if before['decision_loss']-best_metrics['decision_loss'] < POLICY['minimum_improvement']:
        best, best_metrics = zero, before
    held_after = singer_metrics(best, subsets[1], units)
    result.update(proposed_deltas_db=best, metrics_predicted=best_metrics, held_out_proposed=held_after)
    failures = singer_regressions(held_after, held, POLICY['held_out_tolerance'])
    if failures:
        result['reasons'] = ['held-out veto; no refit: '+r for r in failures]
        return finish()
    result['status'] = 'candidate' if any(best) else 'no_change'
    result['objective_met'] = all(m['decision_loss'] <= 1e-12 and all(
        v is None or v <= POLICY['combined_ceiling_db'] for v in m['combined_max_db'])
        for m in (best_metrics, held_after))
    result['reasons'] = ['bounded solo-anchored improvement' if any(best) else
                         'objective met' if result['objective_met'] else 'bounded search unresolved']
    return finish(best)


def accept_singers(proposal, baseline, actual, composition, split):
    result = copy.deepcopy(proposal)
    result.update(proposed_settings=result.pop('settings'), settings=copy.deepcopy(baseline['settings']),
        accepted=False, status='rejected', validation_policy=copy.deepcopy(VALIDATION),
        composition=composition, validation_reasons=[])
    failures = result['validation_reasons']
    if proposal.get('singer_policy') != SOLO or proposal.get('state_context_policy') != STATE_CONTEXT:
        failures.append('frozen singer or state policy changed')
    result['remeasurement_evidence'] = {}
    for label, measurement in [('baseline', baseline), ('candidate', actual)]:
        result['remeasurement_evidence'][label] = {k: copy.deepcopy(measurement.get(k)) for k in
            ('attribution', 'reasons', 'accepted_windows', 'accepted_seconds', 'confidence', 'activity_evidence')}
        if (measurement.get('state_context_policy') != STATE_CONTEXT
                or measurement.get('singer_policy') != SOLO or measurement.get('units') != proposal['units']
                or measurement.get('activity_evidence') != proposal['activity_evidence']
                or measurement.get('reasons')):
            failures.append(label+': changed or incomplete singer measurement')
    if baseline.get('windows') != proposal['windows']:
        failures.append('baseline replay differs from frozen singer observations')
    old, new = baseline['windows'], actual['windows']
    fields = ('start','frames','active','lineup','state_support','solo')
    if [[e.get(k) for k in fields] for e in old] != [[e.get(k) for k in fields] for e in new]:
        failures.append('candidate singer activity/coverage changed')
    if failures:
        return result
    units, deltas = proposal['units'], proposal['proposed_deltas_db']
    predicted = singer_windows(old, units, deltas)
    errors = [abs(db(a)-db(b)) for p,n in zip(predicted,new)
              for pp,nn in zip(p['power'],n['power']) for a,b in zip(pp,nn)]
    for p,n in zip(old,new):
        errors.extend(abs(db(a)-db(b)) for a,b in zip(p['denominator'],n['denominator']))
        for i,d in enumerate(deltas):
            errors.extend(abs(db(a)+d-db(b)) for a,b in zip(p['singer_power'][i],n['singer_power'][i])
                          if max(a,b) > 1e-18)
    measured_metrics = {}
    for label, select in [('training', lambda e:e['start']+e['frames'] <= split),
                          ('held_out', lambda e:e['start'] >= split)]:
        before = singer_metrics((0.,)*len(units), list(filter(select,old)), units)
        prediction = singer_metrics(deltas, list(filter(select,old)), units)
        measured = singer_metrics((0.,)*len(units), list(filter(select,new)), units)
        measured_metrics[label] = measured
        errors.extend(abs(a-b) for aa,bb in zip(prediction['combined_windows_db'],measured['combined_windows_db'])
                      for a,b in zip(aa,bb))
        tolerance = POLICY['held_out_tolerance'] if label == 'held_out' else VALIDATION['loss_tolerance']
        failures.extend(label+': '+r for r in singer_regressions(measured,before,tolerance))
        if label == 'training' and any(deltas) and before['decision_loss']-measured['decision_loss'] < POLICY['minimum_improvement']-1e-6:
            failures.append('insufficient actual singer improvement')
    result['objective_met'] = all(m['decision_loss'] <= 1e-12 and all(
        v is None or v <= POLICY['combined_ceiling_db'] for v in m['combined_max_db'])
        for m in measured_metrics.values())
    result.update(prediction_error_db=max(errors, default=0.), metrics_resulting=measured_metrics,
                  resulting_windows=new, resulting_attribution=actual.get('attribution'))
    if result['prediction_error_db'] > VALIDATION['prediction_tolerance_db']:
        failures.append('production singer prediction error exceeds frozen tolerance')
    if composition.get('passed') is not True:
        failures.append('production composition protection failed')
    if not failures:
        result.update(accepted=True, settings=result['proposed_settings'],
                      status='accepted' if any(deltas) else 'accepted_no_change')
    for decision in result['decisions']:
        decision['accepted_db'] = result['settings']['channels'][decision['channel']]['fader_db']
    return result


# Frozen before production evaluation: numerical replay tolerance, not a fitted target.
VALIDATION = dict(version='vocal-replay-v1', prediction_tolerance_db=0.01,
                  loss_tolerance=1e-6, combined_tolerance_db=1e-6)


def accept_remeasurement(proposal, baseline, actual, composition, split):
    if proposal.get('singer_policy'):
        return accept_singers(proposal, baseline, actual, composition, split)
    result = copy.deepcopy(proposal)
    candidate = result.pop('settings')
    result.update(proposed_settings=candidate, settings=copy.deepcopy(baseline['settings']),
                  accepted=False, validation_policy=copy.deepcopy(VALIDATION),
                  composition=composition, validation_reasons=[], prediction_error_db=None)
    failures = result['validation_reasons']
    result['status'] = 'rejected'
    if proposal.get('state_context_policy') != STATE_CONTEXT:
        failures.append('frozen spectral state protection policy changed')
    result['remeasurement_evidence'] = {}
    for label, measured in [('baseline', baseline), ('candidate', actual)]:
        if measured.get('state_context_policy') != proposal.get('state_context_policy'):
            failures.append(label+': spectral state protection policy differs from frozen proposal')
        result['remeasurement_evidence'][label] = {k: copy.deepcopy(measured.get(k)) for k in
            ('attribution', 'reasons', 'accepted_windows', 'accepted_seconds', 'confidence', 'activity_evidence')}
        if measured['metrics_before'] is None or 'held_out_before' not in measured:
            failures.append(label+': incomplete production evidence: '+ '; '.join(measured.get('reasons', [])))
    if failures:
        return result
    old, new = baseline['windows'], actual['windows']
    if proposal.get('activity_evidence') != baseline.get('activity_evidence'):
        failures.append('baseline per-microphone eligibility differs from frozen observations')
    if baseline.get('activity_evidence') != actual.get('activity_evidence'):
        failures.append('candidate per-microphone eligibility changed')
    if old != proposal['windows']:
        failures.append('baseline replay differs from frozen observations')
    if [(e['start'], e['frames'], e['active'], e['lineup'], e['state_support']) for e in old] != [(e['start'], e['frames'], e['active'], e['lineup'], e['state_support']) for e in new]:
        failures.append('candidate coverage/activity changed')
        return result
    deltas = proposal['proposed_deltas_db']
    errors = []
    group_errors = [[0., 0.] for _ in range(2)]
    combined_errors = [0., 0.]
    for a, b in zip(old, new):
        for i in range(2):
            if a['active'][i]:
                for j in range(2):
                    error = abs(db(b['power'][i][j])-db(b['denominator'][j]) -
                                (db(a['power'][i][j])-db(a['denominator'][j])+deltas[i]))
                    group_errors[i][j] = max(group_errors[i][j], error)
                    errors.append(error)
    actual_metrics = {}
    predicted_metrics = {}
    for label, select in [('training', lambda e: e['start']+e['frames'] <= split),
                          ('held_out', lambda e: e['start'] >= split)]:
        before = metrics((0., 0.), list(filter(select, old)))
        prediction = metrics(deltas, list(filter(select, old)))
        measured = metrics((0., 0.), list(filter(select, new)))
        actual_metrics[label] = measured
        predicted_metrics[label] = prediction
        for j, (predicted, observed) in enumerate(zip(prediction['combined_windows_db'], measured['combined_windows_db'])):
            differences = [abs(a-b) for a, b in zip(predicted, observed)]
            combined_errors[j] = max(combined_errors[j], max(differences, default=0.))
            errors.extend(differences)
        tolerance = POLICY['held_out_tolerance'] if label == 'held_out' else VALIDATION['loss_tolerance']
        context_tolerance = CONTEXT['held_out_tolerance'] if label == 'held_out' else CONTEXT['replay_tolerance']
        for key in context_regressions(measured, before, context_tolerance):
            failures.append(label+': actual lineup loss regression; no refit: '+key)
        state_tolerance = STATE_CONTEXT['held_out_tolerance'] if label == 'held_out' else STATE_CONTEXT['replay_tolerance']
        for key in state_regressions(measured, before, state_tolerance):
            failures.append(label+': actual spectral state loss regression; no refit: '+key)
        if any(a > b+tolerance for a, b in zip(measured['group_loss'], before['group_loss'])):
            failures.append(label+': actual group loss regression; no refit')
        if label == 'training' and any(deltas) and before['loss']-measured['loss'] < POLICY['minimum_improvement']-VALIDATION['loss_tolerance']:
            failures.append('insufficient actual training improvement')
        if any(a > max(POLICY['combined_ceiling_db'], b)+VALIDATION['combined_tolerance_db']
               for aa, bb in zip(measured['combined_windows_db'], before['combined_windows_db']) for a, b in zip(aa, bb)):
            failures.append(label+': actual combined ceiling regression')
    result['objective_met'] = all(m['loss'] <= 1e-12 and all(v <= POLICY['combined_ceiling_db'] for v in m['combined_max_db']) for m in actual_metrics.values())
    result['resulting_attribution'] = actual.get('attribution')
    result['resulting_windows'] = new
    result['prediction_errors_db'] = dict(lead=group_errors[0], backing=group_errors[1], combined=combined_errors)
    result['metrics_resulting'] = actual_metrics
    result['remeasured_prediction'] = predicted_metrics
    result['prediction_error_db'] = max(errors, default=0.)
    if result['prediction_error_db'] > VALIDATION['prediction_tolerance_db']:
        failures.append('production prediction error exceeds frozen tolerance')
    if composition.get('passed') is not True:
        failures.append('production composition protection failed')
    if not failures:
        result.update(accepted=True, settings=candidate,
                      status='accepted' if any(deltas) else 'accepted_no_change')
    for decision in result['decisions']:
        decision['accepted_db'] = result['settings']['channels'][decision['channel']]['fader_db']
    return result


def validate_production(state, metadata, capture, proposal, source, analyzer):
    baseline_settings = copy.deepcopy(state['baseline'])
    proposal['accepted'] = False
    proposal['composition_policy'] = copy.deepcopy(state['request'].get('composition'))
    if proposal['status'] not in ('candidate', 'no_change'):
        proposal['proposed_settings'] = copy.deepcopy(proposal['settings'])
        if 'proposed_deltas_db' in proposal:
            groups = ([u['channels'] for u in proposal['units']] if proposal.get('singer_policy')
                      else [capture['groups'][name]['channels'] for name in NAMES[:2]])
            for i, channels in enumerate(groups):
                for c in channels:
                    proposal['proposed_settings']['channels'][c]['fader_db'] += proposal['proposed_deltas_db'][i]
        return proposal
    try:
        with tempfile.TemporaryDirectory(prefix='gigpies-vocal-') as temp:
            temp = Path(temp)
            write_json(temp/'state.json', state)
            write_json(temp/'candidate.json', proposal['settings'])
            subprocess.run([str(Path(analyzer).resolve()), 'ensemble-remeasure', str(temp/'state.json'),
                            str(temp/'candidate.json'), str(Path(source).resolve()), str(temp/'replay')],
                           check=True, capture_output=True, text=True)
            measured = []
            for name in ('baseline', 'candidate'):
                path = temp/'replay'/f'{name}-observations.json'
                data = read_metadata(path)
                if (any(data[k] != metadata[k] for k in ('frames', 'sample_rate', 'stage_names', 'band_edges_hz'))
                        or data.get('observation_groups', []) != metadata.get('observation_groups', [])):
                    raise ValueError('production metadata changed')
                measured.append(calculate(state, data, array_items(path, 'windows'), capture, measure_only=True))
            composition = json.loads((temp/'replay'/'composition.json').read_text())
            return accept_remeasurement(proposal, *measured, composition, capture['training_end_frame'])
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
        proposal.update(proposed_settings=proposal['settings'], settings=baseline_settings,
                        status='rejected', validation_reasons=['incomplete production validation: '+detail],
                        validation_policy=copy.deepcopy(VALIDATION))
        return proposal


def read_metadata(path):
    with path.open() as stream:
        prefix = stream.read(65536)
    return json.loads(prefix[:prefix.index('"windows":')].rstrip().rstrip(',')+'}')


def run(analysis, capture_path, output, source=None, analyzer=None):
    if Path(output).exists():
        raise FileExistsError(output)
    analysis = Path(analysis)
    state = json.loads((analysis/'state.json').read_text())
    path = analysis/'observations.json'
    metadata = read_metadata(path)
    capture = json.loads(Path(capture_path).read_text())
    result = calculate(state, metadata, array_items(path, 'windows'), capture)
    if source is None and state['baseline'].get('effects') is not None:
        result.update(proposed_settings=copy.deepcopy(result['settings']),
                      settings=copy.deepcopy(state['baseline']), accepted=False)
        if result['status'] in ('candidate', 'no_change'):
            result.update(status='rejected', validation_reasons=['FX acceptance requires --source production replay'])
    if source is not None:
        result = validate_production(state, metadata, capture, result, source, analyzer)
    if 'proposed_settings' in result:
        for decision in result['decisions']:
            decision['proposed_db'] = result['proposed_settings']['channels'][decision['channel']]['fader_db']
            decision['proposed_delta_db'] = decision['proposed_db']-decision['before_db']
            decision['accepted_db'] = result['settings']['channels'][decision['channel']]['fader_db']
            decision['accepted_provenance'] = (decision['provenance_after'] if result['accepted']
                                               else decision['provenance_before'])
    # Validate complete serialization before creating a partially usable candidate.
    json.dumps(result, allow_nan=False)
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    write_json(output/'settings.json', result.pop('settings'))
    write_json(output/'decision.json', result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('analysis', type=Path)
    parser.add_argument('capture', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--analyzer', type=Path, default=Path('target/debug/gigpies'))
    args = parser.parse_args()
    run(args.analysis, args.capture, args.output, args.source, args.analyzer)
