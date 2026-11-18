import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from vocal_balance import POLICY, calculate, run, accept_remeasurement, select_fx_bus


def fixture(lead_db=-3, backing_db=-3, coherent=0., singers=1):
    edges = [0, 25, 100, 120, 350, 400, 1000, 1600, 3200, 5000, 8000, 12000, 24000]
    import math
    pink = [1e-8 if a == 0 else 1e-4*math.log2(b/a) for a, b in zip(edges, edges[1:])]
    n = singers+2
    back = list(range(1, singers+1))
    channels = [dict(file=f'{i}.wav', fader_db=0.) for i in range(n)]
    profiles = [] if singers == 1 else [dict(name='backs', channels=back)]
    state = dict(baseline=dict(channels=channels, sample_rate=48000, effects=None),
                 request=dict(observation_groups=profiles), frozen=dict(id='frozen', inputs=[]))
    meta = dict(stage_names=['raw', '', '', '', 'routed_before_excitation_fx_master'],
                sample_rate=48000, frames=96000, band_edges_hz=edges,
                observation_groups=[] if singers == 1 else [('backs', n)])
    def group(cs, obs, singer):
        return dict(channels=cs, observation=obs, performances=[dict(singer=singer+str(c), channels=[c],
            capture='direct', confidence=.9, spans=[[0, 96000]]) for c in cs])
    capture = dict(frozen_id='frozen', training_end_frame=48000, provenance=['inherited']*n,
        groups=dict(lead=group([0], 0, 'lead'), backing=group(back, 1 if singers == 1 else 'backs', 'back'),
                    accompaniment=dict(channels=[n-1], observation=n-1)))
    powers = [[v*10**(d/10) for v in pink] for d in [lead_db, backing_db, 0]]
    rows = []
    for start in range(0, 96000, 4800):
        for i in range(n+(singers > 1)):
            g = 0 if i == 0 else 2 if i == n-1 else 1
            p = powers[g]
            if 1 <= i < n-1 and singers > 1:
                # Coherent identical backing microphones sum as N².
                p = [v/singers**2 for v in p]
            others = [j for j in range(3) if j != g]
            rest = [sum(powers[j][b] for j in others) for b in range(len(pink))]
            if g == 2:
                rest = [v+2*coherent*math.sqrt(powers[0][b]*powers[1][b]) for b, v in enumerate(rest)]
            stage = dict(mean_power=sum(p), band_power=p)
            rows.append(dict(start=start, frames=4800, channel=i, stages=[copy.deepcopy(stage) for _ in range(5)],
                             competition=dict(rest_band_power=rest)))
    return state, meta, rows, capture


class VocalBalanceTests(unittest.TestCase):
    def solve(self, *args, **kwargs):
        return calculate(*fixture(*args, **kwargs))

    def isolated(self, data):
        for name in ('lead', 'backing'):
            for p in data[3]['groups'][name]['performances']:
                p.pop('spans')
                p['activity'] = dict(mode='isolated_voice', attestation='voice_only_no_bleed',
                                     review_basis='Controlled isolated synthetic source')
        return data

    def contrast(self, singers=1):
        data = fixture(-8, singers=singers)
        # Four seconds: 1 s calibration, 1 s decision training, 2 s held-out.
        data[1]['frames'] = 192000
        data[3]['training_end_frame'] = 96000
        rows = copy.deepcopy(data[2])
        for row in rows:
            row['start'] += 96000
        data[2].extend(rows)
        for name in ('lead', 'backing'):
            for p in data[3]['groups'][name]['performances']:
                p['spans'] = [[0, 192000]]
        p = data[3]['groups']['lead']['performances'][0]
        p.pop('spans')
        p['activity'] = dict(mode='spectral_contrast', calibration_end_frame=48000,
            assumption='stable_capture_spectral_separation', review_basis='Known generated classes',
            examples=[dict(span=[0,24000], label='direct'), dict(span=[24000,48000], label='bleed'),
                      dict(span=[96000,120000], label='bleed'), dict(span=[120000,144000], label='direct')])
        for row in data[2]:
            if row['channel'] == 0:
                b = 3 if 24000 <= row['start'] < 48000 or 96000 <= row['start'] < 120000 else 8
                power = [0.]*12
                power[b] = .01
                row['stages'][0] = dict(mean_power=.01, band_power=power)
        return data

    def spectral_states(self, singers=1):
        data = self.contrast(singers)
        # Eight seconds: four supported spectral states, each with five full
        # calibration and held-out windows. Training has no reviewed examples.
        original = copy.deepcopy(data[2])
        for row in original:
            row['start'] += 192000
        data[2].extend(original)
        data[1]['frames'] = 384000
        data[3]['training_end_frame'] = 192000
        for p in data[3]['groups']['backing']['performances']:
            p['spans'] = [[0, 384000]]
        activity = data[3]['groups']['lead']['performances'][0]['activity']
        activity.update(mode='spectral_states', assumption='stable_capture_spectral_states',
                        calibration_end_frame=96000)
        states = [('direct', 'a', 8), ('direct', 'b', 7),
                  ('bleed', 'a', 3), ('bleed', 'b', 4)]
        activity['examples'] = [dict(span=[base+i*24000, base+(i+1)*24000],
                                    label=label, state=state)
            for base in (0, 192000) for i, (label, state, _) in enumerate(states)]
        for row in data[2]:
            if row['channel'] == 0:
                t = row['start']
                band = states[(t % 192000)//24000][2] if t % 192000 < 96000 else 8
                power = [0.]*12
                power[band] = .01
                row['stages'][0] = dict(mean_power=.01, band_power=power)
        return data

    def test_states_support_multiple_shapes_without_interpolation(self):
        data = self.spectral_states()
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        record = result['activity_evidence'][0]['spectral_contrast']
        self.assertEqual(len(record['templates']), 4)
        self.assertFalse(record['reasons'])
        self.assertEqual(record['policy']['version'], 'capture-spectral-states-v1')
        self.assertTrue(all(x['windows'] == 5 for x in record['held_out'].values()))
        self.assertEqual(calculate(*data), result)
        # The single-template mode must still refuse this multimodal capture.
        old = copy.deepcopy(data)
        activity = old[3]['groups']['lead']['performances'][0]['activity']
        activity.update(mode='spectral_contrast', assumption='stable_capture_spectral_separation')
        for e in activity['examples']: e.pop('state')
        self.assertEqual(calculate(*old)['status'], 'refused')
        # A mixture between two direct states is not automatically direct.
        for row in data[2]:
            if row['channel'] == 0 and 96000 <= row['start'] < 120000:
                row['stages'][0]['band_power'] = [0.]*7+[.005, .005]+[0.]*3
        mixed = calculate(*data)
        self.assertEqual(mixed['activity_evidence'][0]['counts']['ambiguous_spectrum'], 5)
        self.assertEqual(mixed['activity_evidence'][0]['training']['windows'], 15)

    def state_balance(self, held_only=False, short=False):
        data = self.spectral_states()
        for offset in range(0, len(data[2]), 3):
            rows = data[2][offset:offset+3]
            t = rows[0]['start']
            protected = (not held_only and 96000 <= t < (100800 if short else 120000)
                         or 216000 <= t < 240000)
            if protected:
                rows[0]['stages'][0]['band_power'] = [0.]*7+[.01]+[0.]*4
                old = rows[0]['stages'][4]['band_power']
                new = [x*10**(.6) for x in old]  # -8 to the unchanged -2 dB upper bound.
                rows[0]['stages'][4]['band_power'] = new
                for r in rows[1:]:
                    r['competition']['rest_band_power'] = [x+b-a for x, a, b in
                        zip(r['competition']['rest_band_power'], old, new)]
        return data

    def test_state_protection_blocks_loss_hidden_by_same_lineup(self):
        from vocal_balance import metrics, context_regressions, state_regressions
        for short in (False, True):
            data = self.state_balance(short=short)
            result = calculate(*data)
            training = [e for e in result['windows'] if 96000 <= e['start'] < 192000]
            before, raised = metrics((0, 0), training), metrics((3, 0), training)
            self.assertLess(raised['loss'], before['loss'])
            self.assertFalse(context_regressions(raised, before, 1e-9))
            self.assertTrue(state_regressions(raised, before, 1e-9))
            self.assertEqual(result['status'], 'no_change')
            self.assertEqual(result['settings'], data[0]['baseline'])
            self.assertFalse(result['objective_met'])
            self.assertEqual(len(before['state_losses']), 2)
            self.assertEqual(calculate(*data), result)

    def test_unseen_training_state_context_can_veto_without_refit(self):
        result = calculate(*self.state_balance(held_only=True))
        self.assertEqual(result['status'], 'refused')
        self.assertGreater(result['proposed_deltas_db'][0], 0)
        self.assertFalse(result['held_out_context_vetoes'])
        self.assertTrue(result['held_out_state_vetoes'])
        self.assertLess(result['held_out_proposed']['loss'], result['held_out_before']['loss'])

    def test_state_membership_replay_and_original_evidence_are_required(self):
        data = self.state_balance()
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        changed = copy.deepcopy(baseline)
        next(e for e in changed['windows'] if e['state_support'][0])['state_support'][0] = []
        for old, new in ((baseline, changed), (changed, changed)):
            result = accept_remeasurement(proposal, old, new, {'passed': True}, 192000)
            self.assertFalse(result['accepted'])
            self.assertEqual(result['settings'], data[0]['baseline'])
            self.assertTrue(result['validation_reasons'])

    def test_actual_state_loss_is_checked_inside_prediction_tolerance(self):
        data = self.state_balance(short=True)
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        for e in actual['windows']:
            if e['start'] == 96000:
                e['power'][0] = [x*10**(.002/10) for x in e['power'][0]]
        result = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 192000)
        self.assertFalse(result['accepted'])
        self.assertLess(result['prediction_error_db'], .01)
        self.assertTrue(any('actual spectral state loss regression' in r for r in result['validation_reasons']))
        self.assertFalse(any('actual lineup loss regression' in r for r in result['validation_reasons']))

    def test_state_protection_policy_cannot_change_in_agreeing_replays(self):
        data = self.state_balance()
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        changed = copy.deepcopy(baseline)
        changed['state_context_policy']['held_out_tolerance'] = 1.
        result = accept_remeasurement(proposal, changed, changed, {'passed': True}, 192000)
        self.assertFalse(result['accepted'])
        self.assertTrue(any('policy differs from frozen proposal' in r for r in result['validation_reasons']))
        proposal['state_context_policy'] = copy.deepcopy(changed['state_context_policy'])
        result = accept_remeasurement(proposal, changed, changed, {'passed': True}, 192000)
        self.assertFalse(result['accepted'])
        self.assertIn('frozen spectral state protection policy changed', result['validation_reasons'])

    def test_overlapping_supports_all_protected_without_state_assignment(self):
        data = self.spectral_states()
        for row in data[2]:
            if row['channel'] == 0 and row['stages'][0]['band_power'][7] > 0:
                row['stages'][0]['band_power'] = [0.]*8+[.01]+[0.]*3
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        for e in result['windows']:
            if e['active'][0]:
                self.assertEqual(len(e['state_support'][0][0][1]), 2)
            else:
                self.assertEqual(e['state_support'][0], [])
        losses = list(result['metrics_before']['state_losses'].values())
        self.assertEqual(len(losses), 2)
        self.assertEqual(losses[0]['frames'], losses[1]['frames'])
        self.assertEqual(losses[0]['loss'], losses[1]['loss'])

    def test_state_contexts_do_not_authorize_ambiguous_bleed_or_idle_windows(self):
        for kind in ('ambiguous', 'bleed', 'idle'):
            data = self.spectral_states()
            for row in data[2]:
                if row['channel'] == 0 and row['start'] == 96000:
                    row['stages'][0]['band_power'] = [0.]*6+[.01]+[0.]*5 if kind == 'ambiguous' else [0.]*3+[.01]+[0.]*8
                    if kind == 'idle': row['stages'][0]['mean_power'] = 0.
            result = calculate(*data)
            window = next(e for e in result['windows'] if e['start'] == 96000)
            self.assertFalse(window['active'][0])
            self.assertEqual(window['state_support'][0], [])

    def test_state_support_is_per_microphone_with_alternates_doubles_and_overrides(self):
        data = self.spectral_states(singers=3)
        performances = data[3]['groups']['backing']['performances']
        performances[1].update(singer=performances[0]['singer'], capture='double', channels=[2, 3])
        performances.pop()
        activity = data[3]['groups']['lead']['performances'][0]['activity']
        raw = {r['start']: r['stages'][0] for r in data[2] if r['channel'] == 0}
        for p in performances:
            p.pop('spans')
            p['activity'] = copy.deepcopy(activity)
        for row in data[2]:
            if row['channel'] in (1, 2, 3):
                row['stages'][0] = copy.deepcopy(raw[row['start']])
                if row['channel'] == 3:
                    power = row['stages'][0]['band_power']
                    power[7], power[8] = power[8], power[7]
        data[3]['provenance'][2] = 'operator_override'
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        self.assertTrue(all(d['delta_db'] == 0 for d in result['decisions'][1:]))
        contexts = list(result['held_out_before']['state_losses'].values())
        self.assertEqual({r['channel'] for r in contexts}, {0, 1, 2, 3})
        self.assertEqual(len(contexts), 8)
        self.assertEqual(result['evidence'], data[3])
        # Same state names belong to different microphone templates, never one
        # averaged model or independently adjusted alternate/double capture.
        records = result['activity_evidence']
        self.assertNotEqual(records[2]['spectral_contrast']['templates'],
                            records[3]['spectral_contrast']['templates'])

    def mixture_capture(self, weight=.5, outside=0.):
        data = self.spectral_states()
        for row in data[2]:
            if row['channel'] == 0 and row['stages'][0]['band_power'][3:5] == [0., 0.]:
                power = [0.]*12
                power[3], power[4] = weight*(1-outside**2), (1-weight)*(1-outside**2)
                power[8] = outside**2
                row['stages'][0]['band_power'] = [.01*x for x in power]
        return data

    def test_bleed_mixtures_can_match_direct_despite_pairwise_separation(self):
        from vocal_balance import SpectralContrast
        for weight in (.25, .5, .75):
            data = self.mixture_capture(weight)
            result = calculate(*data)
            record = result['activity_evidence'][0]['spectral_contrast']
            direct = [c for k, c in record['templates'].items() if record['state_classes'][k] == 'direct']
            bleed = [c for k, c in record['templates'].items() if record['state_classes'][k] == 'bleed']
            self.assertTrue(all(SpectralContrast.distance(a, b) > .30 for a in direct for b in bleed))
            self.assertEqual(result['status'], 'refused')
            self.assertEqual(result['settings'], data[0]['baseline'])
            self.assertTrue(any('bleed-power mixture enclosure' in r for r in result['reasons']))
            self.assertTrue(all(x == 0 for x in record['bleed_mixture_exclusion']['direct_distance_lower_bounds'].values()))

    def test_mixture_guard_includes_support_variation_and_direct_radius(self):
        import math
        # Center-only mixtures have no band 8. A safe guard must also cover
        # bleed support variation AND the entire direct acceptance ball.
        for outside, expected in ((.28, 'refused'), (2*math.sqrt(2)*.1, 'refused'), (.30, 'candidate')):
            result = calculate(*self.mixture_capture(outside=outside))
            self.assertEqual(result['status'], expected)
        # One unsafe direct state vetoes even with another well-separated state.
        data = self.mixture_capture()
        for row in data[2]:
            if row['channel'] == 0 and row['start'] in range(24000, 48000, 4800):
                row['stages'][0]['band_power'] = [0.]*7+[.01]+[0.]*4
        result = calculate(*data)
        bounds = result['activity_evidence'][0]['spectral_contrast']['bleed_mixture_exclusion']['direct_distance_lower_bounds']
        self.assertEqual(sum(x > .1 for x in bounds.values()), 1)
        self.assertEqual(result['status'], 'refused')

    def test_mixture_enclosure_contains_varied_multistate_power_mixtures(self):
        import math
        from vocal_balance import SpectralContrast
        data = self.spectral_states()
        record = calculate(*data)['activity_evidence'][0]['spectral_contrast']
        box = record['bleed_mixture_exclusion']
        # Non-center support points, unequal mixture weights, and limiting
        # single-state weights. This is enclosure mechanics, not recognition.
        for variation in (0., .05, .1):
            features = []
            for band in (3, 4):
                f = [0.]*12
                f[band], f[6] = math.sqrt(1-variation**2), variation
                center = [float(i == band) for i in range(12)]
                self.assertLessEqual(SpectralContrast.distance(f, center), .1)
                features.append(f)
            for weight in (0., .1, .5, .9, 1.):
                mixed = [math.sqrt(weight*a*a+(1-weight)*b*b) for a, b in zip(*features)]
                self.assertTrue(all(lo <= x <= hi for x, lo, hi in
                    zip(mixed, box['feature_lower'], box['feature_upper'])))

    def test_mixture_guard_freezes_before_held_out_and_replay(self):
        data = self.spectral_states()
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        changed = copy.deepcopy(baseline)
        changed['activity_evidence'][0]['spectral_contrast']['bleed_mixture_exclusion']['feature_upper'][0] += .01
        for old, new in ((baseline, changed), (changed, changed)):
            rejected = accept_remeasurement(proposal, old, new, {'passed': True}, 192000)
            self.assertFalse(rejected['accepted'])
            self.assertEqual(rejected['settings'], data[0]['baseline'])
        for row in data[2]:
            if row['channel'] == 0 and row['start'] >= 192000:
                row['stages'][0]['band_power'] = [.001]*12
        after = calculate(*data)
        self.assertEqual(after['status'], 'refused')
        self.assertEqual(after['activity_evidence'][0]['spectral_contrast']['bleed_mixture_exclusion'],
                         proposal['activity_evidence'][0]['spectral_contrast']['bleed_mixture_exclusion'])

    def test_mixture_exclusion_is_per_microphone_and_preserves_locked_controls(self):
        data = self.spectral_states(singers=2)
        p = data[3]['groups']['backing']['performances'][0]
        p.pop('spans')
        p['channels'] = [1, 2]  # Alternate captures of one backing performance.
        data[3]['groups']['backing']['performances'].pop()
        p['activity'] = copy.deepcopy(data[3]['groups']['lead']['performances'][0]['activity'])
        safe = {r['start']: r['stages'][0] for r in data[2] if r['channel'] == 0}
        unsafe = {r['start']: r['stages'][0] for r in self.mixture_capture()[2] if r['channel'] == 0}
        for row in data[2]:
            if row['channel'] in (1, 2):
                row['stages'][0] = copy.deepcopy((safe if row['channel'] == 1 else unsafe)[row['start']])
        for override in (False, True):
            if override: data[3]['provenance'][2] = 'operator_override'
            result = calculate(*data)
            self.assertEqual(result['status'], 'refused')
            self.assertEqual(result['settings'], data[0]['baseline'])
            records = [e['spectral_contrast'] for e in result['activity_evidence']]
            self.assertFalse(records[0]['reasons'])
            self.assertFalse(records[1]['reasons'])
            self.assertTrue(any('bleed-power mixture enclosure' in r for r in records[2]['reasons']))

    def test_states_each_state_requires_held_out_support_without_refit(self):
        for fault in ('sparse', 'wrong_same_class', 'wrong_class', 'below_floor', 'unseen'):
            data = self.spectral_states()
            before = calculate(*data)['activity_evidence'][0]['spectral_contrast']['templates']
            if fault == 'sparse':
                data[3]['groups']['lead']['performances'][0]['activity']['examples'][5]['span'][1] -= 4800
            for row in data[2]:
                if row['channel'] == 0 and 216000 <= row['start'] < 240000:
                    if fault == 'below_floor': row['stages'][0]['mean_power'] = 0.
                    elif fault != 'sparse':
                        power = [0.]*12
                        power[{'wrong_same_class': 8, 'wrong_class': 3, 'unseen': 6}[fault]] = .01
                        row['stages'][0]['band_power'] = power
            result = calculate(*data)
            self.assertEqual(result['status'], 'refused', fault)
            self.assertEqual(result['settings'], data[0]['baseline'])
            self.assertEqual(result['activity_evidence'][0]['spectral_contrast']['templates'], before)

    def test_states_all_cross_class_pairs_must_separate(self):
        data = self.spectral_states()
        for row in data[2]:
            if row['channel'] == 0 and 72000 <= row['start'] < 96000:
                row['stages'][0]['band_power'] = [0.]*7+[.01]+[0.]*4
        result = calculate(*data)
        self.assertEqual(result['status'], 'refused')
        self.assertTrue(any('ambiguous direct/bleed' in r for r in result['reasons']))

    def test_states_same_class_overlap_and_names_do_not_change_actions(self):
        data = self.spectral_states()
        for row in data[2]:
            if row['channel'] == 0 and row['start'] % 192000 < 48000:
                row['stages'][0]['band_power'] = [0.]*8+[.01]+[0.]*3
        before = calculate(*data)
        self.assertEqual(before['status'], 'candidate')
        for e in data[3]['groups']['lead']['performances'][0]['activity']['examples']:
            e['state'] = 'renamed '+e['state']
        self.assertEqual(calculate(*data)['decisions'], before['decisions'])

    def test_reviewed_silent_calibration_cannot_hide_behind_sufficient_coverage(self):
        for data in (self.contrast(), self.spectral_states()):
            a = data[3]['groups']['lead']['performances'][0]['activity']
            cutoff = a['calibration_end_frame']
            extra = dict(span=[cutoff, cutoff+4800], label='direct')
            if a['mode'] == 'spectral_states': extra['state'] = 'a'
            a['calibration_end_frame'] += 4800
            a['examples'].append(extra)
            a['examples'].sort(key=lambda e: e['span'][0])
            for row in data[2]:
                if row['channel'] == 0 and row['start'] == cutoff:
                    row['stages'][0]['mean_power'] = 0.
            result = calculate(*data)
            self.assertEqual(result['status'], 'refused')
            self.assertTrue(any('unusable reviewed calibration' in r for r in result['reasons']))
            record = result['activity_evidence'][0]['spectral_contrast']
            self.assertTrue(all(c['windows'] >= 5 for c in record['calibration'].values()))

    def test_states_calibration_budget_is_per_class_not_per_state(self):
        from unittest.mock import patch
        from vocal_balance import CLASSIFIER
        with patch.dict(CLASSIFIER, maximum_examples=9):
            with self.assertRaisesRegex(ValueError, 'calibration budget'):
                calculate(*self.spectral_states())

    def test_states_idle_training_only_solo_and_unseen_lineups(self):
        for fault in ('idle', 'training_only', 'solo', 'unseen_lineup', 'low_confidence'):
            data = self.spectral_states(singers=2)
            if fault == 'low_confidence':
                data[3]['groups']['lead']['performances'][0]['confidence'] = .79
            for row in data[2]:
                if row['channel'] == 2 and (fault == 'idle' or
                        fault == 'training_only' and row['start'] >= 192000 or
                        fault == 'solo' and row['start'] % 192000 in (100800, 105600) or
                        fault == 'unseen_lineup' and row['start'] in (292800, 297600)):
                    row['stages'][0]['mean_power'] = 0.
            result = calculate(*data)
            if fault in ('idle', 'training_only', 'low_confidence'):
                self.assertEqual(result['status'], 'refused', fault)
                self.assertEqual(result['settings'], data[0]['baseline'])
            else:
                self.assertEqual(result['status'], 'candidate', fault)
                self.assertTrue(any(e['lineup'][1] == [1] for e in result['windows']))
                self.assertGreater(len(result['held_out_before']['context_losses']), 1)

    def test_states_keep_signed_coherence_and_level_invariance(self):
        import math
        for coherence in (-1, 1):
            data = self.spectral_states(singers=2)
            for offset in range(0, len(data[2]), 5):
                rows = data[2][offset:offset+5]
                lead, backing = rows[0]['stages'][4]['band_power'], rows[4]['stages'][4]['band_power']
                rows[3]['competition']['rest_band_power'] = [a+b+2*coherence*math.sqrt(a*b)
                                                            for a, b in zip(lead, backing)]
            result = calculate(*data)
            self.assertEqual(result['status'], 'candidate')
            self.assertTrue(all(coherence*x > 0 for e in result['windows'] for x in e['cross']))
            self.assertTrue(all(x <= 1+1e-9 for x in result['metrics_predicted']['combined_max_db']))
            for row in data[2]:
                row['stages'][0]['mean_power'] *= 2
                row['stages'][0]['band_power'] = [2*x for x in row['stages'][0]['band_power']]
            self.assertEqual(calculate(*data)['decisions'], result['decisions'])

    def test_states_unseen_held_out_lineup_can_veto_balance_without_refit(self):
        data = self.spectral_states(singers=2)
        for offset in range(0, len(data[2]), 5):
            rows = data[2][offset:offset+5]
            solo = rows[0]['start'] in (292800, 297600)
            factor = 10**((1 if solo else -5)/10)
            for i in (1, 2, 4):
                rows[i]['stages'][4]['band_power'] = [p*factor for p in rows[i]['stages'][4]['band_power']]
            if solo: rows[2]['stages'][0]['mean_power'] = 0.
            for i in (0, 3, 4):
                rows[i]['competition']['rest_band_power'] = [
                    sum(rows[j]['stages'][4]['band_power'][b] for j in (0, 3, 4) if j != i)
                    for b in range(12)]
        result = calculate(*data)
        self.assertEqual(result['status'], 'refused')
        self.assertGreater(result['proposed_deltas_db'][1], 0)
        self.assertTrue(result['held_out_context_vetoes'])
        self.assertEqual(result['settings'], data[0]['baseline'])
        self.assertFalse(result['activity_evidence'][0]['spectral_contrast']['reasons'])

    def test_states_schema_and_calibration_coverage(self):
        for fault in ('missing', 'long', 'held_only', 'assumption', 'budget', 'sparse', 'silent'):
            data = self.spectral_states()
            a = data[3]['groups']['lead']['performances'][0]['activity']
            if fault == 'missing': a['examples'][0].pop('state')
            if fault == 'long': a['examples'][0]['state'] = 'x'*65
            if fault == 'held_only': a['examples'][4]['state'] = 'untrained'
            if fault == 'assumption': a['assumption'] = 'stable_capture_spectral_separation'
            if fault == 'budget':
                a['examples'] = [dict(span=[i*4800, (i+1)*4800], label='direct', state=str(i)) for i in range(9)]
            if fault == 'sparse': a['examples'][1]['span'][1] -= 4800
            if fault == 'silent': data[2][0]['stages'][0]['mean_power'] = 0.
            if fault in ('sparse', 'silent'):
                self.assertEqual(calculate(*data)['status'], 'refused')
            else:
                with self.assertRaises(ValueError): calculate(*data)

    def test_states_preserve_group_identity_overrides_and_frozen_evidence(self):
        data = self.spectral_states(singers=3)
        ps = data[3]['groups']['backing']['performances']
        ps[1].update(singer=ps[0]['singer'], capture='double')
        ps[1]['channels'].append(3)
        ps.pop()
        data[3]['provenance'][1] = 'operator_override'
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        self.assertTrue(all(d['delta_db'] == 0 for d in result['decisions'][1:]))
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        model = actual['activity_evidence'][0]['spectral_contrast']
        model['templates'][next(iter(model['templates']))][0] += .01
        rejected = accept_remeasurement(result, actual, actual, {'passed': True}, 192000)
        self.assertFalse(rejected['accepted'])
        self.assertIn('baseline per-microphone eligibility differs from frozen observations',
                      rejected['validation_reasons'])
        # Alternate microphones do not share a fitted state model.
        ps[0].pop('spans')
        ps[0]['activity'] = copy.deepcopy(data[3]['groups']['lead']['performances'][0]['activity'])
        self.assertEqual(calculate(*data)['status'], 'refused')

    def test_contrast_classifies_unlabelled_windows_and_replays(self):
        data = self.contrast()
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        record = result['activity_evidence'][0]
        self.assertEqual(record['counts']['contrast_direct'], 25)
        self.assertEqual(record['counts']['contrast_bleed'], 5)
        self.assertFalse(record['spectral_contrast']['reasons'])
        self.assertEqual(record['training']['windows'], 10)
        self.assertEqual(calculate(*data), result)
        actual = copy.deepcopy(data)
        for row in actual[2]:
            # Change routed coherent contributions; raw classification stays fixed.
            i = row['channel']
            if i < 2:
                factor = 10**(result['proposed_deltas_db'][i]/10)
                row['stages'][4]['band_power'] = [v*factor for v in row['stages'][4]['band_power']]
        for offset in range(0, len(actual[2]), 3):
            rows = actual[2][offset:offset+3]
            for row in rows:
                row['competition']['rest_band_power'] = [sum(r['stages'][4]['band_power'][b]
                    for r in rows if r is not row) for b in range(12)]
        accepted = accept_remeasurement(result, calculate(*data, measure_only=True),
            calculate(*actual, measure_only=True), {'passed': True}, 96000)
        self.assertTrue(accepted['accepted'], accepted['validation_reasons'])

    def test_contrast_ambiguous_unseen_and_wrong_held_out_refuse(self):
        for fault in ('ambiguous', 'unseen', 'wrong', 'idle', 'missing', 'low_confidence', 'room'):
            data = self.contrast()
            p = data[3]['groups']['lead']['performances'][0]
            if fault == 'missing': p['activity']['examples'] = p['activity']['examples'][:2]
            if fault == 'low_confidence': p['confidence'] = .79
            if fault == 'room': p['capture'] = 'room'
            for row in data[2]:
                if row['channel'] != 0: continue
                start = row['start']
                if (fault == 'ambiguous' and 24000 <= start < 48000 or
                        fault == 'wrong' and 96000 <= start < 120000):
                    row['stages'][0]['band_power'] = [0.]*8+[.01]+[0.]*3
                if fault == 'unseen' and start >= 48000:
                    row['stages'][0]['band_power'] = [0.]*6+[.01]+[0.]*5
                if fault == 'idle' and start >= 96000:
                    row['stages'][0]['mean_power'] = 0.
            result = calculate(*data)
            self.assertEqual(result['status'], 'refused', fault)
            self.assertEqual(result['settings'], data[0]['baseline'])

    def test_contrast_templates_ignore_held_out_and_level(self):
        data = self.contrast()
        before = calculate(*data)
        for row in data[2]:
            if row['channel'] == 0:
                row['stages'][0]['band_power'] = [v*3 for v in row['stages'][0]['band_power']]
                row['stages'][0]['mean_power'] *= 3
        self.assertEqual(calculate(*data)['decisions'], before['decisions'])
        for row in data[2]:
            if row['channel'] == 0 and row['start'] >= 96000:
                row['stages'][0]['band_power'] = [.001]*12
        after = calculate(*data)
        self.assertEqual(after['activity_evidence'][0]['spectral_contrast']['templates'],
                         before['activity_evidence'][0]['spectral_contrast']['templates'])
        self.assertEqual(after['status'], 'refused')

    def test_contrast_mixtures_and_variable_calibration_abstain(self):
        for interval in ((0, 24000), (48000, 96000)):
            data = self.contrast()
            for row in data[2]:
                if row['channel'] == 0 and interval[0] <= row['start'] < interval[1]:
                    if row['start'] % 9600 == 0:
                        row['stages'][0]['band_power'] = [0.]*3+[.005]+[0.]*4+[.005]+[0.]*3
            result = calculate(*data)
            if interval[0] == 0:
                self.assertEqual(result['status'], 'refused')
                self.assertTrue(any('variability' in x for x in result['reasons']))
            else:
                self.assertEqual(result['activity_evidence'][0]['counts']['ambiguous_spectrum'], 5)
                self.assertEqual(result['activity_evidence'][0]['training']['windows'], 5)

    def test_contrast_alternate_mics_need_independent_templates(self):
        data = self.contrast(singers=2)
        # Reuse the lead's activity protocol for the alternate backing pair,
        # whose raw spectra cannot discriminate the two reviewed classes.
        ps = data[3]['groups']['backing']['performances']
        ps[0]['channels'] = [1, 2]
        ps.pop()
        ps[0].pop('spans')
        ps[0]['activity'] = copy.deepcopy(data[3]['groups']['lead']['performances'][0]['activity'])
        result = calculate(*data)
        self.assertEqual(result['status'], 'refused')
        self.assertEqual(result['settings'], data[0]['baseline'])
        for row in result['activity_evidence'][1:]:
            self.assertIn('ambiguous direct/bleed calibration spectra', row['spectral_contrast']['reasons'])

    def test_contrast_schema_and_sparse_calibration(self):
        for fault in ('overlap', 'training_label', 'spans', 'assumption', 'sparse'):
            data = self.contrast()
            p = data[3]['groups']['lead']['performances'][0]
            a = p['activity']
            if fault == 'overlap': a['examples'][1]['span'][0] = 0
            if fault == 'training_label': a['examples'][1]['span'] = [48000, 72000]
            if fault == 'spans': p['spans'] = [[0, 4800]]
            if fault == 'assumption': a['assumption'] = 'voice_role'
            if fault == 'sparse':
                a['examples'][0]['span'][1] = 4800
                self.assertEqual(calculate(*data)['status'], 'refused')
            else:
                with self.assertRaises(ValueError): calculate(*data)

    def test_contrast_common_coherent_group_override_and_replay_evidence(self):
        data = self.contrast(singers=3)
        data[3]['groups']['backing']['performances'][1]['singer'] = 'back1'
        data[3]['groups']['backing']['performances'][1]['capture'] = 'double'
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate')
        data[3]['provenance'][0] = 'operator_override'
        result = calculate(*data)
        self.assertEqual(result['decisions'][0]['delta_db'], 0)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        actual['activity_evidence'][0]['spectral_contrast']['templates']['direct'][0] += .01
        rejected = accept_remeasurement(result, baseline, actual, {'passed': True}, 96000)
        self.assertFalse(rejected['accepted'])
        self.assertIn('candidate per-microphone eligibility changed', rejected['validation_reasons'])
        # A refitted replay baseline must not pass simply because both replay
        # runs agree with each other and the group/window mask is unchanged.
        rejected = accept_remeasurement(result, actual, actual, {'passed': True}, 96000)
        self.assertFalse(rejected['accepted'])
        self.assertIn('baseline per-microphone eligibility differs from frozen observations',
                      rejected['validation_reasons'])

    def test_isolated_capture_matches_reviewed_without_annotation(self):
        for singers, coherent in ((1, 0), (3, 1), (3, -1)):
            data = fixture(-8, singers=singers, coherent=coherent)
            reviewed = calculate(*data)
            automatic = calculate(*self.isolated(data))
            self.assertEqual(automatic['decisions'], reviewed['decisions'])
            self.assertEqual(automatic['windows'], reviewed['windows'])
            self.assertEqual(automatic['policy_sha256'], reviewed['policy_sha256'])
            self.assertTrue(all(e['counts'] == {'isolated_energy': 20}
                                for e in automatic['activity_evidence']))

    def test_isolation_is_not_inferred_and_unsupported_capture_abstains(self):
        for kind in ('unknown', 'bleed', 'room', 'low_confidence', 'unreviewed'):
            data = self.isolated(fixture(-8))
            p = data[3]['groups']['lead']['performances'][0]
            if kind == 'low_confidence': p['confidence'] = .79
            elif kind == 'unreviewed': p.pop('activity')
            else: p['capture'] = kind
            r = calculate(*data)
            self.assertEqual(r['status'], 'refused', kind)
            self.assertEqual(r['settings'], data[0]['baseline'])
            reason = ('low_review_confidence' if kind == 'low_confidence' else
                      'unreviewed_energy' if kind == 'unreviewed' else 'unsupported_capture')
            self.assertEqual(r['activity_evidence'][0]['counts'], {reason: 20})

    def test_alternate_mics_and_doubles_keep_common_move_and_override(self):
        data = self.isolated(fixture(-8, -9, singers=3))
        ps = data[3]['groups']['backing']['performances']
        ps[0]['channels'] = [1, 2]  # Alternate microphones of one performance.
        ps.pop(1)
        ps[1]['singer'] = ps[0]['singer']
        ps[1]['capture'] = 'double'  # Separate take, same singer.
        r = calculate(*data)
        moves = [d['delta_db'] for d in r['decisions'] if d['group'] == 'backing']
        self.assertEqual(len(set(moves)), 1)
        self.assertGreater(moves[0], 0)
        data[3]['provenance'][2] = 'operator_override'
        r = calculate(*data)
        self.assertTrue(all(d['delta_db'] == 0 for d in r['decisions'] if d['group'] == 'backing'))
        self.assertEqual(r['settings']['channels'][-1], data[0]['baseline']['channels'][-1])

    def test_isolation_requires_unambiguous_attestation(self):
        for fault in ('attestation', 'review_basis', 'mode', 'spans'):
            data = self.isolated(fixture())
            p = data[3]['groups']['lead']['performances'][0]
            if fault == 'spans': p['spans'] = [[0, 96000]]
            else: p['activity'][fault] = ''
            with self.assertRaises(ValueError): calculate(*data)

    def test_each_microphone_requires_both_splits(self):
        for automatic in (False, True):
            data = fixture(-8, singers=2)
            if automatic: self.isolated(data)
            for row in data[2]:
                if row['channel'] == 2 and row['start'] >= 48000:
                    row['stages'][0]['mean_power'] = 0
            r = calculate(*data)
            self.assertEqual(r['status'], 'refused')
            self.assertEqual(r['settings'], data[0]['baseline'])
            self.assertTrue(any('held_out: idle microphone' in reason and 'channel 2' in reason
                                for reason in r['reasons']))

    def test_automatic_solo_overlap_idle_and_override(self):
        data = self.isolated(fixture(-8))
        for row in data[2]:
            if row['channel'] == 0 and row['start'] % 48000 < 9600:
                row['stages'][0]['mean_power'] = 0
        r = calculate(*data)
        self.assertEqual(r['accepted_windows'], [16, 20])
        self.assertEqual(r['status'], 'candidate')
        data[3]['provenance'][0] = 'operator_override'
        self.assertEqual(calculate(*data)['decisions'][0]['delta_db'], 0)
        for row in data[2]:
            if row['channel'] == 0: row['stages'][0]['mean_power'] = 1e-8
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertEqual(r['activity_evidence'][0]['counts'], {'below_floor': 20})

    def test_automatic_held_out_veto_never_refits(self):
        data = self.isolated(fixture(-8))
        data[2][:] = [r for r in data[2] if r['start'] < 48000] + [
            r for r in fixture(2)[2] if r['start'] >= 48000]
        r = calculate(*data)
        self.assertIn('held-out veto', r['reasons'][0])
        self.assertGreater(r['proposed_deltas_db'][0], 0)
        self.assertEqual(r['settings'], data[0]['baseline'])

    def test_replay_checks_microphone_eligibility_even_if_group_mask_matches(self):
        data = self.isolated(fixture(-8))
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        actual['activity_evidence'][0]['counts']['isolated_energy'] -= 1
        r = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 48000)
        self.assertFalse(r['accepted'])
        self.assertIn('candidate per-microphone eligibility changed', r['validation_reasons'])

    def lineup_fixture(self, held_only=False):
        data = fixture(-3, -8, singers=2)
        solo = fixture(-3, -2, singers=2)
        # A louder solo backing entry occurs twice per split; the other
        # microphone is silent. Both microphones still have sufficient coverage.
        for j, row in enumerate(data[2]):
            if row['start'] % 48000 < 9600 and (not held_only or row['start'] >= 48000):
                data[2][j] = copy.deepcopy(solo[2][j])
                if row['channel'] in (1, 2):
                    factor = 4 if row['channel'] == 1 else 0
                    for stage in data[2][j]['stages']:
                        stage['mean_power'] *= factor
                        stage['band_power'] = [p*factor for p in stage['band_power']]
        return data

    def test_lineup_guard_protects_solo_entry_from_aggregate_boost(self):
        from vocal_balance import metrics, context_regressions
        data = self.lineup_fixture()
        r = calculate(*data)
        training = [e for e in r['windows'] if e['start'] < 48000]
        before, raised = metrics((0, 0), training), metrics((0, 3), training)
        self.assertLess(raised['loss'], before['loss'])
        self.assertTrue(context_regressions(raised, before, 1e-9))
        self.assertEqual(r['decisions'][1]['delta_db'], 0)
        self.assertFalse(r['objective_met'])
        self.assertEqual(len(before['context_losses']), 2)
        self.assertEqual(calculate(*data), r)

    def test_held_out_new_lineup_veto_does_not_refit(self):
        data = self.lineup_fixture(held_only=True)
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertGreater(r['proposed_deltas_db'][1], 0)
        self.assertTrue(r['held_out_context_vetoes'])
        # Overall held-out loss improves, so this is specifically a lineup veto.
        self.assertLess(r['held_out_proposed']['loss'], r['held_out_before']['loss'])
        self.assertEqual(r['settings'], data[0]['baseline'])

    def test_replay_lineup_change_and_context_regression_refuse(self):
        data = self.lineup_fixture()
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        actual['windows'][0]['lineup'][1] = [1, 2]
        r = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 48000)
        self.assertIn('candidate coverage/activity changed', r['validation_reasons'])
        actual = copy.deepcopy(baseline)
        for e in actual['windows']:
            if e['lineup'][1] == [1]:
                e['power'][1] = [p*2 for p in e['power'][1]]
        r = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 48000)
        self.assertFalse(r['accepted'])
        self.assertTrue(any('actual lineup loss regression' in x for x in r['validation_reasons']))
        self.assertEqual(r['settings'], data[0]['baseline'])

    def test_lineups_preserve_alternate_and_double_capture_identity(self):
        data = self.lineup_fixture()
        ps = data[3]['groups']['backing']['performances']
        ps[1]['singer'] = ps[0]['singer']
        ps[1]['capture'] = 'double'
        doubled = calculate(*data)
        ps[0]['channels'] = [1, 2]
        ps.pop()
        alternate = calculate(*data)
        self.assertEqual(doubled['windows'], alternate['windows'])
        self.assertEqual(doubled['decisions'], alternate['decisions'])
        data[3]['provenance'][1] = 'operator_override'
        self.assertEqual(calculate(*data)['settings'], data[0]['baseline'])

    def test_known_deficiency_and_excess(self):
        for level, sign in [(-8, 1), (2, -1)]:
            r = self.solve(lead_db=level)
            self.assertEqual(r['status'], 'candidate')
            self.assertGreater(sign*r['decisions'][0]['delta_db'], 0)
            self.assertLess(r['metrics_predicted']['loss'], r['metrics_before']['loss'])
            self.assertTrue(r['objective_met'])

    def test_balanced_preserved_and_deterministic(self):
        data = fixture(); before = copy.deepcopy(data)
        a = calculate(*data)
        self.assertEqual(a, calculate(*data))
        self.assertEqual(data, before)
        self.assertEqual(a['status'], 'no_change')
        self.assertTrue(all(d['delta_db'] == 0 for d in a['decisions']))

    def test_inherited_offset_not_a_target(self):
        data = fixture(backing_db=-9)
        data[0]['baseline']['channels'][1]['fader_db'] = -6
        r = calculate(*data)
        self.assertGreater(r['decisions'][1]['delta_db'], 0)
        self.assertEqual(r['decisions'][1]['provenance_after'], 'solver_selected')

    def test_override_preserved(self):
        data = fixture(backing_db=-9); data[3]['provenance'][1] = 'operator_override'
        r = calculate(*data)
        self.assertEqual(r['decisions'][1]['delta_db'], 0)
        self.assertFalse(r['objective_met'])

    def test_coherent_multiple_singers_uses_sum(self):
        r = self.solve(singers=3)
        self.assertEqual(r['status'], 'no_change')
        r = self.solve(backing_db=-9, singers=3)
        ds = [d['delta_db'] for d in r['decisions'] if d['group'] == 'backing']
        self.assertEqual(len(set(ds)), 1)
        self.assertGreater(ds[0], 0)

    def test_combined_coherence_constrains_joint_move(self):
        r = self.solve(-8, -8, coherent=1)
        self.assertLessEqual(max(r['metrics_predicted']['combined_max_db']), 1+1e-9)
        self.assertGreater(sum(d['delta_db'] for d in r['decisions']), 0)

    def test_sparse_bleed_and_missing_refuse(self):
        for change in ['sparse', 'bleed', 'missing', 'idle']:
            data = fixture(-9)
            p = data[3]['groups']['lead']['performances'][0]
            if change == 'sparse': p['spans'] = [[0, 4800]]
            if change == 'bleed': p['capture'] = 'bleed'
            if change == 'missing': p['spans'] = []
            if change == 'idle':
                for row in data[2]:
                    if row['channel'] == 0: row['stages'][0]['mean_power'] = 0
            r = calculate(*data)
            self.assertEqual(r['status'], 'refused', change)
            self.assertTrue(all(d['delta_db'] == 0 for d in r['decisions']))

    def test_legacy_post_fader_fx_observations_refused(self):
        data = fixture(-9); data[0]['baseline']['effects'] = dict(post_fader=True)
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertIn('post-fader', r['reasons'][0])

    def test_fx_attribution_contract_and_tail_refusals(self):
        from vocal_balance import ATTRIBUTION, FX_STAGE
        for fault in (None, 'method', 'tolerance', 'coverage', 'tail', 'nonfinite', 'reconstruction'):
            data = fixture(-8)
            data[0]['baseline']['effects'] = {'buses': []}
            data[0]['request']['vocal_fx_attribution'] = True
            data[1]['stage_names'][4] = FX_STAGE
            a = dict(ATTRIBUTION, checked_frames=96000, max_absolute_error=0.,
                     max_tolerance_fraction=0., tail_peak=0., tail_complete=True)
            data[1]['attribution'] = a
            if fault == 'method': a['method'] = 'nonlinear-marginal'
            if fault == 'tolerance': a['absolute_tolerance'] = 1.
            if fault == 'coverage': a['checked_frames'] -= 1
            if fault == 'tail': a['tail_complete'] = False
            if fault == 'nonfinite': a['max_absolute_error'] = float('nan')
            if fault == 'reconstruction': a['max_tolerance_fraction'] = 1.01
            r = calculate(*data)
            self.assertEqual(r['status'], 'candidate' if fault is None else 'refused')
            if fault: self.assertEqual(r['settings'], data[0]['baseline'])

    def test_fx_policy_preserves_review_overrides_coherence_and_held_out(self):
        from vocal_balance import ATTRIBUTION, FX_STAGE
        for case in ('balanced', 'correction', 'override', 'bleed', 'sparse', 'missing', 'held_out', 'cancellation'):
            data = fixture(-3 if case == 'balanced' else -8, singers=3,
                           coherent=-1 if case == 'cancellation' else 0)
            if case == 'override': data[3]['provenance'][0] = 'operator_override'
            if case == 'bleed': data[3]['groups']['lead']['performances'][0]['capture'] = 'bleed'
            if case == 'sparse': data[3]['groups']['lead']['performances'][0]['spans'] = [[0,4800]]
            if case == 'missing': data[3]['groups']['lead']['performances'][0]['spans'] = []
            if case == 'held_out':
                held = fixture(2, singers=3)
                data[2][:] = [r for r in data[2] if r['start'] < 48000]+[r for r in held[2] if r['start'] >= 48000]
            dry = calculate(*data)
            data[0]['baseline']['effects'] = {'buses': []}
            data[0]['request']['vocal_fx_attribution'] = True
            data[1]['stage_names'][4] = FX_STAGE
            data[1]['attribution'] = dict(ATTRIBUTION, checked_frames=96000, max_absolute_error=0.,
                                        max_tolerance_fraction=0., tail_peak=0., tail_complete=True)
            wet = calculate(*data)
            self.assertEqual(wet['status'], dry['status'], case)
            self.assertEqual(wet['decisions'], dry['decisions'], case)
            if case in ('bleed', 'sparse', 'missing', 'held_out'):
                self.assertEqual(wet['settings'], data[0]['baseline'])

    def test_fx_proposal_without_replay_keeps_baseline(self):
        from vocal_balance import ATTRIBUTION, FX_STAGE
        state, meta, rows, capture = fixture(-8)
        state['baseline']['effects'] = {'buses': []}
        state['request']['vocal_fx_attribution'] = True
        meta['stage_names'][4] = FX_STAGE
        meta['attribution'] = dict(ATTRIBUTION, checked_frames=96000, max_absolute_error=0.,
                                  max_tolerance_fraction=0., tail_peak=0., tail_complete=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); analysis = root/'analysis'; analysis.mkdir()
            (analysis/'state.json').write_text(json.dumps(state))
            (analysis/'observations.json').write_text(json.dumps(dict(**meta, windows=rows), indent=2))
            (root/'capture.json').write_text(json.dumps(capture))
            r = run(analysis, root/'capture.json', root/'result')
            self.assertFalse(r['accepted'])
            self.assertEqual(json.loads((root/'result/settings.json').read_text()), state['baseline'])
            self.assertNotEqual(r['proposed_settings'], state['baseline'])

    def test_dense_context_and_common_scale(self):
        data = fixture(-8, -8)
        a = calculate(*data)
        for row in data[2]:
            for stage in row['stages']:
                stage['mean_power'] *= 10
                stage['band_power'] = [x*10 for x in stage['band_power']]
            row['competition']['rest_band_power'] = [x*10 for x in row['competition']['rest_band_power']]
        b = calculate(*data)
        self.assertEqual([d['delta_db'] for d in a['decisions']], [d['delta_db'] for d in b['decisions']])

    def test_held_out_veto_without_refit(self):
        data = fixture(-8)
        held = fixture(2)
        data[2][:] = [r for r in data[2] if r['start'] < 48000]+[r for r in held[2] if r['start'] >= 48000]
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertIn('held-out veto', r['reasons'][0])
        self.assertGreater(r['proposed_deltas_db'][0], 0)
        self.assertEqual(r['decisions'][0]['delta_db'], 0)

    def test_bad_identity_partition_and_nonfinite_rejected(self):
        for kind in ['identity', 'partition', 'nan', 'window']:
            data = fixture()
            if kind == 'identity': data[3]['frozen_id'] = 'stale'
            if kind == 'partition': data[3]['groups']['backing']['channels'] = [0]
            if kind == 'nan': data[2][0]['stages'][4]['band_power'][1] = float('nan')
            if kind == 'window': data[2].pop()
            with self.assertRaises(ValueError): calculate(*data)

    def test_quiet_words_and_tone_conflict_refuse_level_fix(self):
        data = fixture(-12)
        louder = fixture(3)
        # Both quiet and strong phrases occur in training and validation.
        data[2][:] = [r for r in data[2] if r['start'] % 9600 == 0]+[r for r in louder[2] if r['start'] % 9600 != 0]
        data[2].sort(key=lambda r: (r['start'], r['channel']))
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertTrue(any('dynamics' in x for x in r['reasons']))
        data = fixture(-3)
        # Bright accompaniment forces an articulation deficit while broad
        # voice contribution is excessive. Rebuild consistent complements.
        for offset in range(0, len(data[2]), 3):
            rows = data[2][offset:offset+3]
            for b in range(len(data[1]['band_edges_hz'])-1):
                lo = data[1]['band_edges_hz'][b]
                rows[0]['stages'][4]['band_power'][b] *= 100 if lo < 1600 else .01
                for i in range(3):
                    rows[i]['competition']['rest_band_power'][b] = sum(rows[j]['stages'][4]['band_power'][b] for j in range(3) if i != j)
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertTrue(any('tone review' in x for x in r['reasons']))

    def test_single_idle_member_prevents_group_boost(self):
        data = fixture(backing_db=-9, singers=2)
        for row in data[2]:
            if row['channel'] == 2:
                row['stages'][0]['mean_power'] = 0
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertTrue(any('idle microphone' in x for x in r['reasons']))

    def test_bounds_and_singer_identity(self):
        data = fixture(-20)
        r = calculate(*data)
        self.assertEqual(r['decisions'][0]['delta_db'], 6)
        self.assertFalse(r['objective_met'])
        data[3]['groups']['backing']['performances'][0]['singer'] = 'lead0'
        with self.assertRaises(ValueError): calculate(*data)

    def test_independent_activity_and_solo_backing_entries(self):
        data = fixture(-8, -8)
        p = data[3]['groups']['lead']['performances'][0]
        p['spans'] = [[9600, 48000], [57600, 96000]]
        for row in data[2]:
            if row['channel'] == 0 and row['start'] % 48000 < 9600:
                row['stages'][0]['mean_power'] = 0
        r = calculate(*data)
        self.assertEqual(r['accepted_windows'], [16, 20])
        self.assertEqual(r['status'], 'candidate')
        self.assertEqual(len(r['metrics_before']['combined_windows_db'][0]), 8)

    def test_absent_routed_voice_not_boosted(self):
        data = fixture(-200)
        for row in data[2]:
            if row['channel'] == 0: row['stages'][0]['mean_power'] = .01
        r = calculate(*data)
        self.assertEqual(r['status'], 'refused')
        self.assertTrue(any('missing routed' in x for x in r['reasons']))

    def test_fixed_channel_dsp_is_not_recomputed(self):
        data = fixture(-8)
        data[0]['baseline']['channels'][0].update(compressor={'ratio': 8}, dynamic_eq={'enabled': True})
        r = calculate(*data)
        self.assertEqual(r['status'], 'candidate')
        self.assertEqual(r['settings']['channels'][0]['compressor'], {'ratio': 8})
        self.assertEqual(r['settings']['channels'][0]['dynamic_eq'], {'enabled': True})

    def test_production_acceptance_and_vetoes(self):
        for singers, coherent, level in [(1, 0., -8), (3, 1., -8), (1, 0., -3)]:
            data = fixture(level, -3, coherent, singers)
            proposal = calculate(*data)
            baseline = calculate(*data, measure_only=True)
            delta = proposal['proposed_deltas_db']
            actual_data = fixture(level+delta[0], -3+delta[1], coherent, singers)
            actual = calculate(*actual_data, measure_only=True)
            accepted = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 48000)
            self.assertTrue(accepted['accepted'], accepted)
            self.assertLess(accepted['prediction_error_db'], .01)
            for fault in ('nonlinear', 'held_out', 'composition', 'missing', 'timeline'):
                changed = copy.deepcopy(actual)
                composition = {'passed': fault != 'composition'}
                if fault in ('nonlinear', 'held_out'):
                    for e in changed['windows']:
                        if fault == 'nonlinear' or e['start'] >= 48000:
                            e['power'][0] = [p*100 for p in e['power'][0]]
                if fault == 'missing': changed['metrics_before'] = None
                if fault == 'timeline': changed['windows'].pop()
                rejected = accept_remeasurement(proposal, baseline, changed, composition, 48000)
                self.assertFalse(rejected['accepted'], fault)
                self.assertEqual(rejected['settings'], data[0]['baseline'])

    def test_incomplete_replay_preserves_attribution_failure_evidence(self):
        data = fixture(-8)
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        actual.update(metrics_before=None, reasons=['incomplete FX tail'],
                      attribution={'tail_complete': False, 'tail_peak': .01})
        result = accept_remeasurement(proposal, baseline, actual, {'passed': True}, 48000)
        self.assertFalse(result['accepted'])
        self.assertEqual(result['settings'], data[0]['baseline'])
        self.assertIn('incomplete FX tail', result['validation_reasons'][0])
        self.assertEqual(result['remeasurement_evidence']['candidate']['attribution'], actual['attribution'])

    def test_override_and_fx_replay_boundaries(self):
        from vocal_balance import validate_production
        data = fixture(-3, -9)
        data[3]['provenance'][1] = 'operator_override'
        proposal = calculate(*data)
        measured = calculate(*data, measure_only=True)
        result = accept_remeasurement(proposal, measured, measured, {'passed': True}, 48000)
        self.assertTrue(result['accepted'])
        self.assertEqual(result['settings'], data[0]['baseline'])
        self.assertFalse(result['objective_met'])
        for kind in ('sparse', 'bleed', 'fx'):
            data = fixture(-8)
            if kind == 'fx': data[0]['baseline']['effects'] = {'post_fader': True}
            if kind == 'bleed': data[3]['groups']['lead']['performances'][0]['capture'] = 'bleed'
            if kind == 'sparse': data[3]['groups']['lead']['performances'][0]['spans'] = []
            result = validate_production(data[0], data[1], data[3], calculate(*data), '.', '/nonexistent')
            self.assertEqual(result['status'], 'refused')
            self.assertEqual(result['settings'], data[0]['baseline'])
            self.assertFalse(result['accepted'])

    def test_failed_replay_retains_baseline(self):
        from vocal_balance import validate_production
        data = fixture(-8)
        proposal = calculate(*data)
        result = validate_production(data[0], data[1], data[3], proposal, '.', '/nonexistent/gigpies')
        self.assertEqual(result['status'], 'rejected')
        self.assertFalse(result['accepted'])
        self.assertEqual(result['settings'], data[0]['baseline'])
        self.assertNotEqual(result['proposed_settings'], result['settings'])

    def test_cli_artifacts_and_no_overwrite(self):
        state, meta, rows, capture = fixture(-8)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); analysis = root/'analysis'; analysis.mkdir()
            (analysis/'state.json').write_text(json.dumps(state))
            (analysis/'observations.json').write_text(json.dumps(dict(**meta, windows=rows), indent=2))
            (root/'capture.json').write_text(json.dumps(capture))
            r = run(analysis, root/'capture.json', root/'result')
            self.assertEqual(r['status'], 'candidate')
            self.assertTrue((root/'result'/'settings.json').exists())
            self.assertEqual(r['policy'], POLICY)
            with self.assertRaises(FileExistsError): run(analysis, root/'capture.json', root/'result')
            capture['invalid_extra'] = float('nan')
            (root/'capture.json').write_text(json.dumps(capture))
            with self.assertRaises(ValueError): run(analysis, root/'capture.json', root/'invalid')
            self.assertFalse((root/'invalid').exists())




def singer_fixture(levels=(-8., -8., -2.), linked=False):
    import math
    state, meta, _, capture = fixture(singers=2)
    capture['decision_mode'] = 'solo_anchored'
    state['request']['observation_pairs'] = [[0,1], [0,2], [1,2]]
    # Two backing solo contexts then overlap, in each chronological split.
    meta['frames'] = 144000
    capture['training_end_frame'] = 72000
    for name in ('lead','backing'):
        for p in capture['groups'][name]['performances']:
            p['spans'] = [[0,144000]]
    edges = meta['band_edges_hz']
    pink = [1e-8 if a == 0 else 1e-4*math.log2(b/a) for a,b in zip(edges,edges[1:])]
    rows = []
    for start in range(0,144000,4800):
        context = (start % 72000)//24000
        powers = [[p*10**(d/10) for p in pink] for d in levels]+[pink]
        if context < 2:
            powers[2 if context == 0 else 1] = [0.]*len(pink)
        if linked:
            powers[2] = [x*.25 for x in powers[1]]
        groups = [powers[0], [a+b for a,b in zip(powers[1],powers[2])], pink]
        if linked:
            groups[1] = [x*2.25 for x in powers[1]]
        for c in range(5):
            p = powers[c] if c < 4 else groups[1]
            g = 0 if c == 0 else 2 if c == 3 else 1
            rest = [sum(groups[j][b] for j in range(3) if j != g) for b in range(len(pink))]
            stage = dict(mean_power=sum(p),band_power=p)
            pairs = []
            for a,b in state['request']['observation_pairs']:
                if a == c:
                    pair = [x+y for x,y in zip(powers[a],powers[b])]
                    if linked and (a,b) == (1,2): pair = groups[1]
                    pairs.append([b,pair])
            rows.append(dict(start=start,frames=4800,channel=c,stages=[copy.deepcopy(stage) for _ in range(5)],
                             competition=dict(rest_band_power=rest),pair_sum_band_power=pairs))
    if linked:
        capture['groups']['backing']['performances'][1]['singer'] = capture['groups']['backing']['performances'][0]['singer']
        capture['groups']['backing']['performances'][1]['capture'] = 'double'
    return state,meta,rows,capture


class SoloSingerTests(unittest.TestCase):
    def test_independent_solo_corrections_and_overlap(self):
        data = singer_fixture()
        result = calculate(*data)
        self.assertEqual(result['status'],'candidate',result['reasons'])
        ds = result['proposed_deltas_db']
        self.assertGreater(ds[0],0)
        self.assertGreater(ds[1],0)
        self.assertLess(ds[2],0)
        self.assertLess(result['metrics_predicted']['decision_loss'],result['metrics_before']['decision_loss'])
        self.assertEqual(calculate(*data),result)
        for old,new in zip(result['metrics_before']['anchors'],result['metrics_predicted']['anchors']):
            self.assertLessEqual(new['loss'],old['loss']+1e-9)

    def test_linked_doubles_preserve_offsets_and_override(self):
        data = singer_fixture(linked=True)
        result = calculate(*data)
        self.assertEqual(result['status'],'candidate',result['reasons'])
        self.assertEqual(len(result['units']),2)
        self.assertEqual(result['decisions'][1]['delta_db'],result['decisions'][2]['delta_db'])
        data[3]['provenance'][2] = 'operator_override'
        result = calculate(*data)
        self.assertEqual(result['proposed_deltas_db'][1],0)

    def test_missing_solo_evidence_refuses_no_target_division(self):
        data = singer_fixture()
        data[2][:] = [r for r in data[2] if r['start']%72000 >= 48000]
        result = calculate(*data)
        self.assertEqual(result['status'],'refused')
        self.assertTrue(any('solo anchor' in r for r in result['reasons']))
        self.assertEqual(result['settings'],data[0]['baseline'])

    def test_pairs_required_and_signed_reconstruction_checked(self):
        data = singer_fixture()
        data[0]['request']['observation_pairs'].pop()
        with self.assertRaisesRegex(ValueError,'every routed vocal'):
            calculate(*data)
        data = singer_fixture()
        data[2][0]['pair_sum_band_power'][0][1][3] = 100.
        with self.assertRaisesRegex(ValueError,'inconsistent vocal pair'):
            calculate(*data)
        data = singer_fixture(linked=True)
        data[2][1]['pair_sum_band_power'][0][1] = [0.]*12
        with self.assertRaises(ValueError): calculate(*data)

    def test_held_out_veto_without_refit(self):
        data = singer_fixture()
        held = singer_fixture(levels=(-2.,-8.,-2.))
        data[2][:] = [r for r in data[2] if r['start']<72000]+[r for r in held[2] if r['start']>=72000]
        result = calculate(*data)
        self.assertEqual(result['status'],'refused')
        self.assertTrue(any('held-out veto' in r for r in result['reasons']))
        self.assertEqual(result['settings'],data[0]['baseline'])

    def test_replay_checks_independent_predictions_and_composition(self):
        data = singer_fixture()
        proposal = calculate(*data)
        baseline = calculate(*data,measure_only=True)
        actual = copy.deepcopy(baseline)
        from vocal_balance import singer_windows
        actual['windows'] = copy.deepcopy(singer_windows(baseline['windows'],proposal['units'],proposal['proposed_deltas_db']))
        for row in actual['windows']:
            for i,d in enumerate(proposal['proposed_deltas_db']):
                row['singer_power'][i] = [x*10**(d/10) for x in row['singer_power'][i]]
            for entry in row['singer_cross']:
                entry[2] = [x*10**((proposal['proposed_deltas_db'][entry[0]]+proposal['proposed_deltas_db'][entry[1]])/20) for x in entry[2]]
        accepted = accept_remeasurement(proposal,baseline,actual,dict(passed=True),72000)
        self.assertTrue(accepted['accepted'],accepted['validation_reasons'])
        self.assertFalse(accept_remeasurement(proposal,baseline,actual,dict(passed=False),72000)['accepted'])
        actual['windows'][0]['singer_power'][0][0] *= 2
        self.assertFalse(accept_remeasurement(proposal,baseline,actual,dict(passed=True),72000)['accepted'])

    def test_level_and_identity_invariance_and_bounded_controls(self):
        data = singer_fixture()
        expected = calculate(*data)['proposed_deltas_db']
        for row in data[2]:
            for stage in row['stages']:
                stage['mean_power'] *= .01
                stage['band_power'] = [x*.01 for x in stage['band_power']]
            row['competition']['rest_band_power'] = [x*.01 for x in row['competition']['rest_band_power']]
            row['pair_sum_band_power'] = [[i,[x*.01 for x in p]] for i,p in row['pair_sum_band_power']]
        for name in ('lead','backing'):
            for p in data[3]['groups'][name]['performances']: p['singer'] = 'renamed-'+p['singer']
        self.assertEqual(calculate(*data)['proposed_deltas_db'],expected)
        data[0]['baseline']['channels'][1]['fader_db'] = 12.
        self.assertLessEqual(calculate(*data)['proposed_deltas_db'][1],0.)

    def test_no_change_is_unresolved_when_solo_and_overlap_targets_conflict(self):
        data = singer_fixture(levels=(-3.,-4.,-4.))
        result = calculate(*data)
        self.assertEqual(result['status'],'no_change')
        self.assertFalse(result['objective_met'])
        self.assertEqual(result['settings'],data[0]['baseline'])

    def test_stale_policy_and_changed_accompaniment_fail_replay(self):
        data = singer_fixture(levels=(-3.,-4.,-4.))
        proposal = calculate(*data)
        baseline = calculate(*data,measure_only=True)
        actual = copy.deepcopy(baseline)
        actual['windows'][0]['denominator'][0] *= 1.01
        self.assertFalse(accept_remeasurement(proposal,baseline,actual,dict(passed=True),72000)['accepted'])
        proposal['singer_policy']['maximum_passes'] += 1
        self.assertFalse(accept_remeasurement(proposal,baseline,baseline,dict(passed=True),72000)['accepted'])


def contextual_fixture(levels=(-8., -8., -2.), linked=False):
    data = singer_fixture(levels, linked)
    data[3]['decision_mode'] = 'contextual'
    # Retain only overlap: five windows in each split, no backing solo anchors.
    data[2][:] = [r for r in data[2] if r['start'] % 72000 >= 48000]
    return data


class ContextualSingerTests(unittest.TestCase):
    def test_overlap_only_independent_redistribution(self):
        data = contextual_fixture()
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate', result['reasons'])
        self.assertGreater(result['proposed_deltas_db'][1], 0)
        self.assertLess(result['proposed_deltas_db'][2], 0)
        self.assertEqual(calculate(*data), result)
        for old,new in zip(result['metrics_before']['singers'], result['metrics_predicted']['singers']):
            self.assertLessEqual(new['loss'], old['loss']+1e-9)
        self.assertFalse(any(w['solo'][1] or w['solo'][2] for w in result['windows']))

    def test_contextual_competitor_uses_signed_other_singer_sum(self):
        from vocal_balance import contextual_metrics
        data = contextual_fixture()
        baseline = calculate(*data, measure_only=True)
        rows = copy.deepcopy(baseline['windows'])
        # Two cancelling competitors with individually large powers must not
        # become a fictitious incoherent masker. Powers are analytical evidence.
        for w in rows:
            w['singer_power'][1] = w['denominator'][:]
            w['singer_power'][2] = w['denominator'][:]
            for pair in w['singer_cross']:
                pair[2] = [-2*v for v in w['denominator']] if pair[:2] == [1,2] else [0.,0.]
        m = contextual_metrics((0.,0.,0.), rows, baseline['units'])
        self.assertAlmostEqual(m['singers'][0]['ratios_db'][0]['p50'], -8.)
        for w in rows:
            for pair in w['singer_cross']:
                if pair[:2] == [1,2]: pair[2] = [2*v for v in w['denominator']]
        m = contextual_metrics((0.,0.,0.), rows, baseline['units'])
        self.assertAlmostEqual(m['singers'][0]['ratios_db'][0]['p50'], -14.0205999133)

    def test_balanced_single_singers_preserve_controls(self):
        result = calculate(*contextual_fixture((-3.,-3.,-3.), linked=True))
        # Linked captures are one singer, not two targets.
        self.assertEqual(len(result['units']), 2)
        self.assertEqual(result['decisions'][1]['delta_db'], result['decisions'][2]['delta_db'])
        data = singer_fixture((-3.,-3.,-3.))
        data[3]['decision_mode'] = 'contextual'
        data[2][:] = [r for r in data[2] if r['start']%72000 < 48000]
        result = calculate(*data)
        self.assertEqual(result['status'], 'no_change')
        self.assertTrue(result['objective_met'])

    def test_held_out_cannot_choose_or_refit_gains(self):
        data = contextual_fixture()
        held = contextual_fixture((-2.,-8.,-2.))
        data[2][:] = [r for r in data[2] if r['start']<72000]+[r for r in held[2] if r['start']>=72000]
        result = calculate(*data)
        self.assertEqual(result['status'], 'refused')
        self.assertTrue(any('held-out veto' in r for r in result['reasons']))
        self.assertEqual(result['settings'], data[0]['baseline'])
        self.assertEqual(result['proposed_deltas_db'], calculate(*contextual_fixture())['proposed_deltas_db'])

    def test_unknown_activity_and_missing_pairs_still_refuse(self):
        data = contextual_fixture()
        data[3]['groups']['backing']['performances'][0]['capture'] = 'unknown'
        self.assertEqual(calculate(*data)['status'], 'refused')
        data = contextual_fixture()
        data[0]['request']['observation_pairs'].pop()
        with self.assertRaisesRegex(ValueError, 'every routed vocal'):
            calculate(*data)

    def test_linked_override_locks_singer_not_other_actions(self):
        data = contextual_fixture(linked=True)
        data[3]['provenance'][2] = 'operator_override'
        result = calculate(*data)
        self.assertEqual(result['proposed_deltas_db'][1], 0.)
        self.assertGreater(result['proposed_deltas_db'][0], 0.)
        self.assertEqual(result['actionability'][1]['reasons'], ['operator override'])

    def test_individual_lineup_regression_cannot_hide_in_aggregate(self):
        from vocal_balance import singer_regressions
        baseline = calculate(*contextual_fixture(), measure_only=True)['metrics_before']
        after = copy.deepcopy(baseline)
        key = next(iter(after['singer_context_losses']))
        after['singer_context_losses'][key] += 1
        self.assertIn('individual singer lineup/state loss regression', singer_regressions(after,baseline,1e-9))

    def test_tone_conflict_freezes_only_unsupported_singer(self):
        data = contextual_fixture()
        # Build physically consistent independent spectra with a dark singer:
        # weak articulation and excessive low-range contribution.
        for start in sorted({r['start'] for r in data[2]}):
            rows = {r['channel']:r for r in data[2] if r['start']==start}
            for b in range(12):
                if data[1]['band_edges_hz'][b] < 1600:
                    for stage in rows[2]['stages']: stage['band_power'][b] *= 2
                else:
                    for stage in rows[2]['stages']: stage['band_power'][b] *= .01
            for stage in rows[2]['stages']: stage['mean_power'] = sum(stage['band_power'])
            powers = [rows[c]['stages'][4]['band_power'] for c in range(4)]
            group = [a+b for a,b in zip(powers[1],powers[2])]
            for stage in rows[4]['stages']:
                stage['band_power'] = group[:]; stage['mean_power'] = sum(group)
            for c,row in rows.items():
                own = {0: [0],1:[1,2],2:[1,2],3:[3],4:[1,2]}[c]
                row['competition']['rest_band_power'] = [sum(powers[j][b] for j in range(4) if j not in own) for b in range(12)]
                row['pair_sum_band_power'] = [[j,[x+y for x,y in zip(powers[c],powers[j])]] for j in range(c+1,3)] if c < 3 else []
        result = calculate(*data)
        self.assertEqual(result['status'], 'candidate', result['reasons'])
        self.assertEqual(result['proposed_deltas_db'][2], 0.)
        self.assertIn('singer contribution/articulation conflict', result['actionability'][2]['reasons'])
        self.assertTrue(any(result['proposed_deltas_db'][:2]))
        self.assertFalse(result['objective_met'])

    def test_contextual_level_invariance_and_bounds(self):
        data = contextual_fixture()
        expected = calculate(*data)['proposed_deltas_db']
        for row in data[2]:
            for stage in row['stages']:
                stage['mean_power'] *= .1
                stage['band_power'] = [v*.1 for v in stage['band_power']]
            row['competition']['rest_band_power'] = [v*.1 for v in row['competition']['rest_band_power']]
            row['pair_sum_band_power'] = [[c,[v*.1 for v in p]] for c,p in row['pair_sum_band_power']]
        self.assertEqual(calculate(*data)['proposed_deltas_db'], expected)
        data[0]['baseline']['channels'][1]['fader_db'] = 12.
        self.assertLessEqual(calculate(*data)['proposed_deltas_db'][1], 0.)

    def test_contextual_state_support_is_protected_individually(self):
        from vocal_balance import contextual_metrics, singer_regressions
        result = calculate(*contextual_fixture(), measure_only=True)
        rows = result['windows']
        rows[0]['state_support'][1] = [[1,['soft','bright']]]
        before = contextual_metrics((0.,0.,0.), rows, result['units'])
        keys = [k for k in before['singer_context_losses'] if 'soft' in k or 'bright' in k]
        self.assertEqual(len(keys), 2)
        after = copy.deepcopy(before)
        after['singer_context_losses'][keys[0]] += .06
        self.assertIn('individual singer lineup/state loss regression', singer_regressions(after,before,.05))

    def test_production_acceptance_checks_contextual_loss_and_policy(self):
        from vocal_balance import singer_windows
        data = contextual_fixture()
        proposal = calculate(*data)
        baseline = calculate(*data, measure_only=True)
        actual = copy.deepcopy(baseline)
        ds = proposal['proposed_deltas_db']
        actual['windows'] = copy.deepcopy(singer_windows(baseline['windows'],proposal['units'],ds))
        for row in actual['windows']:
            for i,d in enumerate(ds):
                row['singer_power'][i] = [x*10**(d/10) for x in row['singer_power'][i]]
            for entry in row['singer_cross']:
                entry[2] = [x*10**((ds[entry[0]]+ds[entry[1]])/20) for x in entry[2]]
        accepted = accept_remeasurement(proposal,baseline,actual,dict(passed=True),72000)
        self.assertTrue(accepted['accepted'], accepted['validation_reasons'])
        self.assertIn('singers', accepted['metrics_resulting']['held_out'])
        tampered = copy.deepcopy(actual)
        # Opposite pair errors preserve both role powers and their combined sum.
        row = tampered['windows'][0]
        amount = row['singer_power'][0][0]*.1
        row['singer_cross'][0][2][0] += amount
        row['singer_cross'][1][2][0] -= amount
        checked = accept_remeasurement(proposal,baseline,tampered,dict(passed=True),72000)
        self.assertFalse(checked['accepted'])
        self.assertGreater(checked['prediction_error_db'], .01)
        actual['windows'][0]['singer_power'][1][0] *= 2
        self.assertFalse(accept_remeasurement(proposal,baseline,actual,dict(passed=True),72000)['accepted'])
        proposal['singer_policy']['ratio_db'][0] = -10
        self.assertFalse(accept_remeasurement(proposal,baseline,baseline,dict(passed=True),72000)['accepted'])



def wet_fixture(wet_db=-4., levels=(0., -3., -3.), linked=False):
    from vocal_balance import ATTRIBUTION, FX_STAGE
    data = contextual_fixture(levels, linked)
    state,meta,rows,capture = data
    capture['decision_mode'] = 'wetness'
    capture['wetness_provenance'] = ['inherited']*4
    state['request']['vocal_fx_attribution'] = True
    state['baseline']['effects'] = dict(buses=[dict(name='shared', sends=[dict(channel=c,db=-12.) for c in range(4)])])
    meta['stage_names'][4] = FX_STAGE
    meta['attribution'] = dict(ATTRIBUTION, checked_frames=meta['frames'], max_tolerance_fraction=0.,
                               max_absolute_error=0., tail_complete=True, tail_peak=0.)
    # Synthetic incoherent direct/return components with an exact known ratio.
    ratio = 10**(wet_db/10)
    for row in rows:
        total = row['stages'][4]['band_power']
        row['vocal_components'] = dict(direct=dict(band_power=[p/(1+ratio) for p in total]),
                                       wet=dict(band_power=[p*ratio/(1+ratio) for p in total]))
    by_time = {}
    for row in rows: by_time.setdefault(row['start'],{})[row['channel']] = row
    for group in by_time.values():
        for c,row in group.items():
            pairs = []
            for other,total in row['pair_sum_band_power']:
                cross = [t-a-b for t,a,b in zip(total,row['stages'][4]['band_power'],group[other]['stages'][4]['band_power'])]
                sums = []
                for left,right in [('direct','direct'),('direct','wet'),('wet','direct'),('wet','wet')]:
                    fraction = (1/(1+ratio) if left == 'direct' else ratio/(1+ratio)) if left == right else 0.
                    sums.append([a+b+x*fraction for a,b,x in zip(row['vocal_components'][left]['band_power'],
                        group[other]['vocal_components'][right]['band_power'],cross)])
                pairs.append([other,sums])
            row['vocal_component_pairs'] = pairs
    return data


def wet_replay(data, proposal):
    """Independent synthetic sum construction; production Rust test covers DSP."""
    result = copy.deepcopy(data)
    factors = [10**(d/10) for d in proposal['wet_deltas_db']]
    for row in result[2]:
        c = row['channel']
        if c == 3:
            continue
        i = 0 if c == 0 else 1
        components = row['vocal_components']
        components['wet']['band_power'] = [p*factors[i] for p in components['wet']['band_power']]
        row['stages'][4]['band_power'] = [a+b for a,b in zip(components['direct']['band_power'],components['wet']['band_power'])]
    by_time = {}
    old_time = {}
    for row in result[2]: by_time.setdefault(row['start'], {})[row['channel']] = row
    for row in data[2]: old_time.setdefault(row['start'], {})[row['channel']] = row
    for start,group in by_time.items():
        power = [group[c]['stages'][4]['band_power'] for c in (0,4,3)]
        for c,row in group.items():
            g = 0 if c == 0 else 2 if c == 3 else 1
            row['competition']['rest_band_power'] = [sum(p[b] for j,p in enumerate(power) if j != g) for b in range(len(power[0]))]
            old = old_time[start][c]
            pair_sums = []
            total_pairs = []
            for other,components in old['vocal_component_pairs']:
                sums,terms = [],[]
                for value,(left,right) in zip(components,[('direct','direct'),('direct','wet'),('wet','direct'),('wet','wet')]):
                    a = old['vocal_components'][left]['band_power']
                    b = old_time[start][other]['vocal_components'][right]['band_power']
                    import math
                    factor = (math.sqrt(factors[0 if c == 0 else 1]) if left == 'wet' else 1.) * (math.sqrt(factors[0 if other == 0 else 1]) if right == 'wet' else 1.)
                    xy = [(v-x-y)*factor for v,x,y in zip(value,a,b)]
                    terms.append(xy)
                    sums.append([a+b+x for a,b,x in zip(row['vocal_components'][left]['band_power'],group[other]['vocal_components'][right]['band_power'],xy)])
                pair_sums.append([other,sums])
                total_pairs.append([other,[a+b+sum(t[k] for t in terms) for k,(a,b) in enumerate(zip(row['stages'][4]['band_power'],group[other]['stages'][4]['band_power']))]])
            row['vocal_component_pairs'] = pair_sums
            row['pair_sum_band_power'] = total_pairs
    return calculate(*result, measure_only=True)


class WetnessTests(unittest.TestCase):
    def test_bus_winner_veto_preserves_baseline_without_refit(self):
        data = wet_fixture()
        state, meta, _, capture = data
        capture['decision_mode'] = 'bus_wetness'
        bus = state['baseline']['effects']['buses'][0]
        bus['name'] = 'delay'
        base = calculate(*data)
        def replay(state, metadata, capture, settings, source, analyzer):
            delta = settings['effects']['buses'][0]['sends'][0]['db'] - bus['sends'][0]['db']
            actual = wet_replay(data, {'wet_deltas_db': [delta, delta]})
            if delta == -6:
                for row in actual['wet_windows']:
                    if row['start'] >= capture['training_end_frame']:
                        row['wet'] = [v*100 for v in row['wet']]
            return (base, actual), {'passed': True}
        with patch('vocal_balance.replay_observations', side_effect=replay):
            result = select_fx_bus(state, meta, capture, base, '.', 'analyzer')
            self.assertEqual(result['bus_action']['delta_db'], -6.)
            self.assertFalse(result['accepted'])
            self.assertIn('role wetness regression', result['reasons'])
            self.assertEqual(result['settings'], state['baseline'])
            capture['wetness_provenance'][0] = 'operator_override'
            result = select_fx_bus(state, meta, capture, calculate(*data), '.', 'analyzer')
            self.assertEqual(len(result['bus_trials']), 1)
            self.assertIsNone(result['bus_action'])

    def test_bounded_wet_correction_preserves_faders_and_shared_band_sends(self):
        data = wet_fixture()
        p = calculate(*data)
        self.assertEqual(p['status'], 'candidate', p['reasons'])
        self.assertEqual(p['wet_deltas_db'], [-6.,-6.])
        self.assertEqual(p['settings']['channels'], data[0]['baseline']['channels'])
        self.assertEqual(p['settings']['effects']['buses'][0]['sends'][-1]['db'], -12.)
        self.assertEqual(calculate(*data),p)
        baseline = calculate(*data, measure_only=True)
        actual = wet_replay(data,p)
        accepted = accept_remeasurement(p,baseline,actual,{'passed':True},72000)
        self.assertTrue(accepted['accepted'], accepted['validation_reasons'])
        self.assertTrue(accepted['objective_met'])
        self.assertLess(accepted['prediction_error_db'], .01)

    def test_no_wet_boost_and_minimal_deadband_step(self):
        for wet,expected in [(-20.,0.),(-10.,0.),(-9.,-1.)]:
            data = wet_fixture(wet)
            p = calculate(*data)
            self.assertEqual(p['wet_deltas_db'],[expected]*2)
            self.assertEqual(p['objective_met'],wet <= -4.)

    def test_linked_captures_and_independent_override(self):
        data = wet_fixture(linked=True)
        data[3]['wetness_provenance'][1] = 'operator_override'
        p = calculate(*data)
        self.assertEqual(p['wet_deltas_db'],[-6.,0.])
        sends = p['settings']['effects']['buses'][0]['sends']
        self.assertEqual([s['db'] for s in sends],[-18.,-12.,-12.,-12.])
        self.assertFalse(p['objective_met'])
        data[3]['provenance'][0] = 'operator_override'
        self.assertEqual(calculate(*data)['wet_deltas_db'],[0.,0.])

    def test_independent_send_with_locked_role_peer(self):
        data = wet_fixture(levels=(0.,-3.,3.))
        data[3]['wetness_provenance'][:2] = ['operator_override']*2
        proposal = calculate(*data)
        self.assertEqual(proposal['wet_individual'], dict(channels=[2], delta=-6.))
        self.assertEqual([s['db'] for s in proposal['settings']['effects']['buses'][0]['sends']],
                         [-12.,-12.,-18.,-12.])

    def test_missing_direct_or_legacy_evidence_refuses(self):
        for fault in ('missing','direct','tail','bleed'):
            data = wet_fixture()
            if fault == 'missing': data[2][0].pop('vocal_components')
            if fault == 'direct': data[2][0]['vocal_components']['direct']['band_power'] = [0.]*12
            if fault == 'tail': data[1]['attribution']['tail_complete'] = False
            if fault == 'bleed': data[3]['groups']['lead']['performances'][0]['capture'] = 'bleed'
            p = calculate(*data)
            self.assertEqual(p['status'],'refused',fault)
            self.assertEqual(p['settings'],data[0]['baseline'])
        data = wet_fixture(); data[3].pop('wetness_provenance')
        with self.assertRaisesRegex(ValueError,'send provenance'): calculate(*data)

    def test_production_vetoes_preserve_baseline_without_refit(self):
        data = wet_fixture(); p = calculate(*data)
        baseline = calculate(*data,measure_only=True)
        for fault in ('direct','wet','held_out','composition','activity','policy','controls','accompaniment','incomplete'):
            actual = wet_replay(data,p); proposal = copy.deepcopy(p)
            if fault in ('direct','wet'):
                actual['wet_windows'][0][fault][0] *= 2
            if fault == 'held_out':
                for e in actual['windows']:
                    if e['start'] >= 72000:
                        e['singer_power'][0] = [v*.01 for v in e['singer_power'][0]]
                        e['power'][0] = [v*.01 for v in e['power'][0]]
            if fault == 'activity': actual['activity_evidence'][0]['counts'] = {}
            if fault == 'incomplete':
                actual['reasons'] = ['incomplete FX tail']
                actual.pop('windows')
            if fault == 'policy': proposal['wetness_policy']['maximum_ratio_db'] = 0.
            if fault == 'controls': proposal['settings']['channels'][0]['fader_db'] = 1.
            if fault == 'accompaniment': actual['windows'][0]['denominator'][0] *= 2
            rejected = accept_remeasurement(proposal,baseline,actual,{'passed':fault != 'composition'},72000)
            self.assertFalse(rejected['accepted'],fault)
            self.assertEqual(rejected['settings'],data[0]['baseline'])

    def test_contribution_conflict_remains_unsupported(self):
        data = wet_fixture(levels=(-8.,-8.,-8.)); p = calculate(*data)
        self.assertEqual(p['status'],'no_change')
        self.assertEqual(p['wet_deltas_db'],[0.,0.])
        self.assertFalse(p['objective_met'])

    def test_cli_wet_proposal_requires_replay(self):
        state,meta,rows,capture = wet_fixture()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); analysis = root/'analysis'; analysis.mkdir()
            (analysis/'state.json').write_text(json.dumps(state))
            (analysis/'observations.json').write_text(json.dumps(dict(**meta,windows=rows),indent=2))
            (root/'capture.json').write_text(json.dumps(capture))
            result = run(analysis,root/'capture.json',root/'out')
            self.assertFalse(result['accepted'])
            self.assertEqual(json.loads((root/'out'/'settings.json').read_text()),state['baseline'])

    def test_joint_search_keeps_smaller_supported_backing_cut(self):
        from vocal_balance import wet_predict, singer_metrics, singer_regressions
        data = wet_fixture(0.)
        p = calculate(*data)
        self.assertEqual(p['wet_deltas_db'],[-6.,-4.5])
        train = [e for e in p['windows'] if e['start']+e['frames'] <= 72000]
        units = p['units']; zero = [0.]*len(units)
        before = singer_metrics(zero,train,units,True)
        full = singer_metrics(zero,wet_predict(train,units,[-6.,-6.]),units,True)
        supported = singer_metrics(zero,wet_predict(train,units,p['wet_deltas_db']),units,True)
        self.assertTrue(singer_regressions(full,before,1e-9))
        self.assertFalse(singer_regressions(supported,before,1e-9))
        self.assertFalse(p['objective_met'])

    def test_held_out_veto_does_not_research_training_selection(self):
        data = wet_fixture(0.)
        p = calculate(*data)
        low = wet_fixture(0.,levels=(-8.,-8.,-8.))
        data[2][:] = [r for r in data[2] if r['start'] < 72000]+[r for r in low[2] if r['start'] >= 72000]
        rejected = calculate(*data)
        self.assertEqual(rejected['status'],'refused')
        self.assertEqual(rejected['wet_deltas_db'],p['wet_deltas_db'])
        self.assertEqual(rejected['settings'],data[0]['baseline'])
        self.assertTrue(any('held-out veto' in r for r in rejected['reasons']))

    def test_component_pairs_are_required_and_physically_bounded(self):
        data = wet_fixture()
        data[2][0]['vocal_component_pairs'] = []
        self.assertEqual(calculate(*data)['status'],'refused')
        data = wet_fixture()
        data[2][0]['vocal_component_pairs'][0][1][0][3] *= 1000
        with self.assertRaisesRegex(ValueError,'component pair'): calculate(*data)

    def test_level_invariance_and_send_floor(self):
        data = wet_fixture(); p = calculate(*data)
        for row in data[2]:
            for stage in row['stages']:
                stage['mean_power'] *= .01
                stage['band_power'] = [v*.01 for v in stage['band_power']]
            row['competition']['rest_band_power'] = [v*.01 for v in row['competition']['rest_band_power']]
            for _,power in row['pair_sum_band_power']:
                power[:] = [v*.01 for v in power]
            for stage in row['vocal_components'].values():
                stage['band_power'] = [v*.01 for v in stage['band_power']]
            for _,components in row['vocal_component_pairs']:
                for power in components: power[:] = [v*.01 for v in power]
        self.assertEqual(calculate(*data)['wet_deltas_db'],p['wet_deltas_db'])
        data[0]['baseline']['effects']['buses'][0]['sends'][0]['db'] = -59.
        self.assertGreaterEqual(calculate(*data)['wet_deltas_db'][0],-1.)

    def test_signed_direct_return_cancellation_is_not_incoherent_addition(self):
        from vocal_balance import wet_predict
        data = wet_fixture()
        # Each direct and its delayed return have opposite phase in this
        # analytical fixture. Channels remain mutually uncorrelated.
        for row in data[2]:
            total = row['stages'][4]['band_power']
            row['vocal_components'] = dict(direct=dict(band_power=[4*p for p in total]),
                                           wet=dict(band_power=list(total)))
        by_time = {}
        for row in data[2]: by_time.setdefault(row['start'],{})[row['channel']] = row
        for group in by_time.values():
            for row in group.values():
                row['vocal_component_pairs'] = [[other,[[a+b for a,b in zip(row['vocal_components'][left]['band_power'],
                    group[other]['vocal_components'][right]['band_power'])]
                    for left,right in [('direct','direct'),('direct','wet'),('wet','direct'),('wet','wet')]]]
                    for other,_ in row['pair_sum_band_power']]
        measured = calculate(*data,measure_only=True)
        self.assertFalse(measured['reasons'])
        prediction = wet_predict(measured['windows'],measured['units'],[-6.,-6.])
        factor = (2-10**(-6/20))**2
        for old,new in zip(measured['windows'],prediction):
            for a,b in zip(old['power'][0],new['power'][0]):
                self.assertAlmostEqual(b,a*factor,places=12)
        # Removing a cancelling return raises total vocals; contribution
        # protection must prevent treating this as a harmless energy reduction.
        self.assertEqual(calculate(*data)['wet_deltas_db'],[0.,0.])


if __name__ == '__main__':
    unittest.main()
