import copy
import json
import math
from pathlib import Path
import tempfile
import unittest

from spectral_balance import array_items, bell_power, calculate, compact, objective, target_curve


class SpectralBalanceTests(unittest.TestCase):
    def fixture(self, excess=False):
        edges = [0,25,100,200,400,800,1600,3200,6400,12000,24000]
        baseline = [1e-8] + [1e-4*math.log2(b/a) for a,b in zip(edges[1:],edges[2:])]
        rows=[]
        for role in ['lead_vocal','rhythm_guitar','bass_di']:
            powers=[x/3 for x in baseline]
            if excess and role=='rhythm_guitar':
                powers[6]*=40
            rows.append(dict(file=role+'.wav',role=role,fader_db=0.,eq=[],
                             active_windows=100,max_reduction_p90_db=0.,band_power=powers))
        settings={'sample_rate':48000,'channels':[{k:v for k,v in r.items() if k in ('file','role','fader_db','eq')} for r in rows]}
        return dict(settings=settings,edges_hz=edges,channels=rows,
                    mix_band_power=[sum(r['band_power'][i] for r in rows) for i in range(len(baseline))])

    def test_flat_per_octave_is_unchanged(self):
        m=self.fixture();r=calculate(m)
        self.assertEqual(r['moves'],[])
        self.assertEqual(r['settings'],m['settings'])

    def test_excess_guitar_calculates_cut_without_touching_vocal(self):
        m=self.fixture(True);before=copy.deepcopy(m);r=calculate(m)
        self.assertTrue(r['moves'])
        self.assertLess(r['objective_predicted'],r['objective_before'])
        self.assertTrue(all(x['channel']==1 for x in r['moves']))
        self.assertEqual(m,before)
        self.assertEqual(r['settings']['channels'][0],m['settings']['channels'][0])
        self.assertGreaterEqual(r['settings']['channels'][1]['fader_db'],-3.)

    def test_common_level_change_does_not_change_choices(self):
        a=self.fixture(True);b=copy.deepcopy(a)
        for row in b['channels']:row['band_power']=[v*10 for v in row['band_power']]
        b['mix_band_power']=[v*10 for v in b['mix_band_power']]
        one,two=calculate(a),calculate(b)
        choices=lambda r:[(x['channel'],x['control'],x['band_hz'],x['delta_db']) for x in r['moves']]
        self.assertEqual(choices(one),choices(two))

    def test_silent_source_is_not_normalized(self):
        m=self.fixture(True);m['channels'][1]['active_windows']=0
        self.assertEqual(calculate(m)['moves'],[])

    def test_no_reference_and_no_input_mutation(self):
        m=self.fixture(True);r=calculate(m,0)
        self.assertEqual(r['settings'],m['settings'])
        self.assertEqual(r['moves'],[])
        with self.assertRaises(ValueError):calculate(m,25)

    def test_compression_prevents_unjustified_eq(self):
        m=self.fixture(True);m['channels'][1]['max_reduction_p90_db']=8.
        r=calculate(m)
        self.assertTrue(all(x['control']=='fader' for x in r['moves']))

    def test_dynamic_processing_prevents_static_transfer_prediction(self):
        m=self.fixture(True)
        m['settings']['channels'][1]['dynamic_eq']={'enabled':True}
        r=calculate(m)
        self.assertTrue(all(x['control']=='fader' for x in r['moves']))

    def test_existing_eq_capacity_is_respected(self):
        m=self.fixture(True);m['settings']['channels'][1]['eq']=[{'hz':100.,'q':1.,'db':0.}]*8
        r=calculate(m)
        self.assertEqual(len(r['settings']['channels'][1]['eq']),8)

    def test_bell_matches_centre_gain_and_zero_bypass(self):
        self.assertAlmostEqual(10*math.log10(bell_power(1000,.8,-3,1000,48000)),-3.,places=9)
        self.assertAlmostEqual(bell_power(1000,.8,0,600,48000),1.,places=12)

    def test_curve_integrates_bandwidth_and_has_no_silence_target(self):
        m=self.fixture();t=target_curve(m['edges_hz'],m['mix_band_power'])
        self.assertAlmostEqual(t[1]/t[2],2.)
        self.assertIsNone(t[0]);self.assertIsNone(t[-1])
        with self.assertRaises(ValueError):target_curve(m['edges_hz'],[0.]*len(t))

    def test_unknown_role_is_not_treated_as_a_disposable_instrument(self):
        m=self.fixture(True)
        m['settings']['channels'][1]['role']='other'
        m['channels'][1]['role']='other'
        self.assertEqual(calculate(m)['moves'],[])

    def test_explicit_family_allows_identified_other_channel(self):
        m=self.fixture(True)
        m['settings']['channels'][1]['role']='other'
        m['channels'][1]['role']='other'
        m['channels'][1]['family']='synth'
        self.assertTrue(calculate(m)['moves'])

    def test_malformed_dimensions_nonfinite_and_identity_fail(self):
        for change in [lambda m:m['mix_band_power'].pop(),
                       lambda m:m['mix_band_power'].__setitem__(0,float('nan')),
                       lambda m:m['channels'][0].__setitem__('file','wrong.wav')]:
            m=self.fixture();change(m)
            with self.assertRaises(ValueError):calculate(m)

    def test_target_is_invariant_to_splitting_a_band(self):
        edges=[0,25,100,400,1600,6400,12000,24000]
        densities=[0,1,2,3,4,5,1]
        power=[1e-5 if a==0 else densities[i]*math.log2(b/a) for i,(a,b) in enumerate(zip(edges,edges[1:]))]
        coarse=target_curve(edges,power)
        fine_edges=[0,25,50,100,200,400,800,1600,3200,6400,12000,24000]
        fine_power=[1e-5 if a==0 else densities[next(i for i,(x,y) in enumerate(zip(edges,edges[1:])) if x<=a<y)]*math.log2(b/a) for a,b in zip(fine_edges,fine_edges[1:])]
        fine=target_curve(fine_edges,fine_power)
        self.assertAlmostEqual(coarse[1]/2,fine[1])

    def test_strong_coherence_is_not_optimized_as_independent_energy(self):
        m=self.fixture(True)
        m['mix_band_power']=[v*3 for v in m['mix_band_power']]
        self.assertEqual(calculate(m)['moves'],[])

    def test_stream_parser_spans_chunks_and_rejects_truncation(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'data.json';rows=[{'text':'x'*80000}, {'value':2}]
            p.write_text(json.dumps({'windows':rows,'other':[]},indent=2))
            self.assertEqual(list(array_items(p,'windows')),rows)
            p.write_text('{"windows": [ {"x": 1}')
            with self.assertRaises(ValueError):list(array_items(p,'windows'))

    def test_coherent_total_reconstruction_and_tail_weight(self):
        # Three identical in-phase channels: total=9*single, not 3*single.
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);analysis=root/'analysis';analysis.mkdir()
            settings={'channels':[{'file':str(i),'role':'other','fader_db':0.} for i in range(3)]}
            (analysis/'state.json').write_text(json.dumps({'baseline':settings,'frozen':{'id':'x','inputs':[]}}))
            rows=[]
            for start,frames,power in [(0,100,1.),(100,1,4.)]:
                for i in range(3):
                    stage={'mean_power':power,'peak':math.sqrt(power),'band_power':[power]}
                    rows.append({'start':start,'frames':frames,'channel':i,'stages':[stage]*5,'max_reduction_db':0.,'competition':{'rest_band_power':[4*power]}})
            (analysis/'observations.json').write_text(json.dumps({'band_edges_hz':[0,24000],'sample_rate':48000,'frames':101,'windows':rows},indent=2))
            m=compact(analysis,root/'compact')
            self.assertAlmostEqual(m['mix_band_power'][0],9*104/101)
            self.assertAlmostEqual(m['channels'][0]['band_power'][0],104/101)
            self.assertEqual(m['channels'][0]['windows'],2)


if __name__=='__main__':unittest.main()
