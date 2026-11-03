#!/usr/bin/env python3
"""Formula-driven spectral balance from production ensemble observations.

No reference recordings. A pink-spectrum target is an explicit engineering
hypothesis, not a claim that all music should have one spectrum. Run `--help`.
"""
import argparse
import copy
import gzip
import html
import json
import math
from pathlib import Path


def db(power):
    return 10 * math.log10(max(power, 1e-24))


def quantile(values, fraction):
    values = sorted(values)
    return values[round((len(values) - 1) * fraction)] if values else None


def array_items(path, key):
    """Read one top-level pretty-printed JSON array without loading the full trace."""
    decoder = json.JSONDecoder()
    with Path(path).open() as stream:
        marker = '"' + key + '": ['
        buffer = ''
        while marker not in buffer:
            chunk = stream.read(65536)
            if not chunk:
                raise ValueError(f'missing array: {key}')
            buffer = buffer[-len(marker):] + chunk
        buffer = buffer.split(marker, 1)[1]
        while True:
            buffer = buffer.lstrip()
            if buffer.startswith(','):
                buffer = buffer[1:].lstrip()
            if buffer.startswith(']'):
                return
            try:
                value, end = decoder.raw_decode(buffer)
            except json.JSONDecodeError:
                chunk = stream.read(65536)
                if not chunk:
                    raise ValueError(f'incomplete array: {key}') from None
                buffer += chunk
                continue
            yield value
            buffer = buffer[end:]


def compact(analysis, out, families=None):
    """Keep reproducible settings, stage summaries and a small compressed trace."""
    analysis, out = Path(analysis), Path(out)
    state = json.loads((analysis / 'state.json').read_text())
    session = state['baseline']
    path = analysis / 'observations.json'
    # Metadata precedes the large arrays in the production writer.
    with path.open() as stream:
        prefix = stream.read(65536)
    prefix = prefix[:prefix.index('"windows":')].rstrip().rstrip(',') + '}'
    metadata = json.loads(prefix)
    edges = metadata['band_edges_hz']
    channels = session['channels']
    families = families or {}
    allowed = {'kick','bass','bass_di','bass_amp','snare','tom','overheads','drum_room',
               'lead_vocal','backing_vocal','vocal_room','rhythm_guitar','lead_guitar',
               'acoustic_guitar','hi_hat','piano','keys','synth','other'}
    if any(name not in {c['file'] for c in channels} or family not in allowed
           for name,family in families.items()):
        raise ValueError('invalid explicit musical-family binding')
    levels = [[] for _ in channels]
    for row in array_items(path, 'windows'):
        if row['channel'] < len(channels):
            levels[row['channel']].append(db(row['stages'][0]['mean_power']))
    floors = [max(-70., (quantile(x, .95) if x else -240.) - 30.) for x in levels]
    sums = [[0.] * (len(edges) - 1) for _ in channels]
    raw_sums = copy.deepcopy(sums)
    active_sums = copy.deepcopy(sums)
    totals = [0.] * (len(edges) - 1)
    counts = [0] * len(channels)
    frame_counts = [0] * len(channels)
    active_frames = [0] * len(channels)
    total_frames = 0
    active_counts = [0] * len(channels)
    frame_counts = [0] * len(channels)
    active_frames = [0] * len(channels)
    total_frames = 0
    reductions = [[] for _ in channels]
    raw_peaks = [0.] * len(channels)
    group = []
    windows = 0
    out.mkdir(parents=True, exist_ok=False)
    with gzip.open(out / 'features.jsonl.gz', 'wt') as trace:
        for row in array_items(path, 'windows'):
            i = row['channel']
            if i >= len(channels):
                continue
            stages = row['stages']
            raw, routed = stages[0], stages[4]
            active = db(raw['mean_power']) > floors[i]
            data = {'start': row['start'], 'frames': row['frames'], 'channel': i,
                    'active': active, 'raw_power': raw['mean_power'],
                    'routed_power': routed['mean_power'],
                    'raw_bands': raw['band_power'], 'bands': routed['band_power'],
                    'rest_bands': row['competition']['rest_band_power'],
                    'max_reduction_db': row['max_reduction_db']}
            trace.write(json.dumps(data, separators=(',', ':'), allow_nan=False) + '\n')
            counts[i] += 1
            frame_counts[i] += row['frames']
            raw_peaks[i] = max(raw_peaks[i], raw['peak'])
            if active:
                active_counts[i] += 1
                active_frames[i] += row['frames']
                reductions[i].append(row['max_reduction_db'])
            for b, (a, v) in enumerate(zip(raw['band_power'], routed['band_power'])):
                raw_sums[i][b] += (a or 0.) * row['frames']
                sums[i][b] += (v or 0.) * row['frames']
                if active:
                    active_sums[i][b] += (v or 0.) * row['frames']
            group.append(data)
            if len(group) == len(channels):
                if len({(r['start'],r['frames']) for r in group}) != 1 or sorted(r['channel'] for r in group) != list(range(len(channels))):
                    raise ValueError('inconsistent source timeline/order')
                if len(channels) < 3:
                    raise ValueError('spectral sum reconstruction requires at least three channels')
                # Sum_i |T-x_i|² = (N-2)|T|² + Sum_i |x_i|².
                # Thus the actual coherent mix spectrum is recoverable from the
                # production analyzer's source and rest-of-mix spectra.
                for b in range(len(totals)):
                    source = sum(r['bands'][b] or 0. for r in group)
                    rest = sum(r['rest_bands'][b] or 0. for r in group)
                    total = (rest - source) / (len(channels) - 2)
                    if total < -1e-8 * max(source, 1e-20):
                        raise ValueError('invalid coherent spectral sum')
                    totals[b] += max(0., total) * group[0]['frames']
                total_frames += group[0]['frames']
                group = []
                windows += 1
    if group or not windows:
        raise ValueError('incomplete spectral windows')
    summaries = []
    for i, c in enumerate(channels):
        summaries.append({'channel': i, 'file': c['file'], 'role': c['role'],
                          'family': families.get(c['file'], c['role']),
                          'fader_db': c['fader_db'], 'activity_floor_dbfs': floors[i],
                          'active_windows': active_counts[i], 'windows': counts[i],
                          'raw_active_level_p50_dbfs': quantile([x for x in levels[i] if x > floors[i]], .5),
                          'raw_peak_dbfs': db(raw_peaks[i] ** 2),
                          'max_reduction_p90_db': quantile(reductions[i], .9),
                          'raw_band_power': [v / max(frame_counts[i], 1) for v in raw_sums[i]],
                          'band_power': [v / max(frame_counts[i], 1) for v in sums[i]],
                          'active_band_power': [v / max(active_frames[i], 1) for v in active_sums[i]]})
    result = {'version': 1, 'frozen_id': state['frozen']['id'], 'settings': session,
              'source_identities': state['frozen']['inputs'],
              'source_observations': metadata.get('channels', []), 'edges_hz': edges,
              'sample_rate': metadata['sample_rate'], 'frames': metadata['frames'],
              'channels': summaries, 'mix_band_power': [v / total_frames for v in totals],
              'scope': 'Continuous production channel DSP; raw and routed spectra before excitation, generated FX and master. Symmetric-Hann FFT window power, not LUFS.',
              'activity_limit': 'Raw energy gate is not voice/bleed recognition. Silence is never normalized.'}
    write_json(out / 'measurement.json', result)
    return result


def write_json(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def protected(role, low, high):
    """Explicit preservation policy, never a claim of detected musical identity."""
    ranges = {'kick': (35., 120.), 'bass_di': (35., 300.), 'bass_amp': (35., 300.), 'bass': (35., 300.),
              'lead_vocal': (120., 5000.), 'backing_vocal': (120., 5000.),
              'snare': (100., 250.), 'tom': (50., 250.),
              'acoustic_guitar': (100., 300.)}
    span = ranges.get(role)
    return span is not None and low < span[1] and high > span[0]


def bell_power(hz, q, cut_db, frequency, rate):
    """Exact RBJ bell magnitude at a frequency (same equations as production DSP)."""
    a = 10 ** (cut_db / 40.)
    w = 2 * math.pi * hz / rate
    alpha = math.sin(w) / (2 * q)
    b = [1 + alpha * a, -2 * math.cos(w), 1 - alpha * a]
    d = [1 + alpha / a, -2 * math.cos(w), 1 - alpha / a]
    z = complex(math.cos(-2 * math.pi * frequency / rate), math.sin(-2 * math.pi * frequency / rate))
    return abs((b[0] + b[1]*z + b[2]*z*z) / (d[0] + d[1]*z + d[2]*z*z)) ** 2


def target_curve(edges, mix):
    eligible = [i for i, (lo, hi) in enumerate(zip(edges, edges[1:]))
                if lo >= 25 and hi <= 12000 and mix[i] > 1e-18]
    density = [db(mix[i] / math.log2(edges[i+1] / edges[i])) for i in eligible]
    if len(density) < 4:
        raise ValueError('insufficient supported mix spectrum')
    # Robust fitted vertical placement; no broadcast target and no output gain.
    weighted = sorted((value, math.log2(edges[i+1]/edges[i]))
                      for value,i in zip(density,eligible))
    half = sum(weight for _,weight in weighted)/2
    cumulative = 0.
    anchor = weighted[-1][0]
    for value,weight in weighted:
        cumulative += weight
        if cumulative >= half-1e-12:
            anchor = value
            break
    target = [None] * len(mix)
    for i in eligible:
        target[i] = 10 ** (anchor / 10) * math.log2(edges[i+1] / edges[i])
    return target


def objective(powers, targets, edges):
    score = 0.
    for i, (value, target) in enumerate(zip(powers, targets)):
        if target is None:
            continue
        # Correct excess, not every dip: no boosting absent musical content.
        excess = max(0., db(value) - db(target) - 3.)
        score += math.log2(edges[i+1] / edges[i]) * excess * excess
    return score


def validate_measurement(m):
    channels = m['channels']
    edges = m['edges_hz']
    settings = m['settings']
    if not 3 <= len(channels) <= 64 or len(channels) != len(settings['channels']):
        raise ValueError('invalid channel dimensions')
    if (not 2 <= len(edges) <= 128 or edges[0] != 0 or
            any(not math.isfinite(x) or x < 0 for x in edges) or
            any(b <= a for a,b in zip(edges,edges[1:]))):
        raise ValueError('invalid spectral edges')
    if not 8000 <= settings['sample_rate'] <= 192000:
        raise ValueError('unsupported sample rate')
    for values in [m['mix_band_power']] + [c['band_power'] for c in channels]:
        if len(values) != len(edges)-1 or any(not math.isfinite(x) or x < 0 for x in values):
            raise ValueError('invalid spectral power')
    for source,channel in zip(channels,settings['channels']):
        if source['file'] != channel['file'] or source['role'] != channel['role']:
            raise ValueError('source identity/order mismatch')
        if not math.isfinite(channel['fader_db']) or not -80 <= channel['fader_db'] <= 12:
            raise ValueError('invalid source fader')
        if source['active_windows'] < 0 or len(channel['eq']) > 8:
            raise ValueError('invalid source evidence/settings')
        reduction = source['max_reduction_p90_db']
        if reduction is not None and (not math.isfinite(reduction) or reduction < 0):
            raise ValueError('invalid compression evidence')


def calculate(measurement, max_steps=12):
    """Choose supported excess-energy reductions, with explicit preservation.

    This first formula policy is subtractive. It never independently normalizes
    singers, raises noise, or claims a spectral score proves preferred balance.
    """
    if not 0 <= max_steps <= 24:
        raise ValueError('invalid step budget')
    m = measurement
    validate_measurement(m)
    s = copy.deepcopy(m['settings'])
    edges, mix = m['edges_hz'], m['mix_band_power']
    if len(m['channels']) != len(s['channels']) or any(not math.isfinite(x) or x < 0 for x in mix):
        raise ValueError('invalid measurement')
    target = target_curve(edges, mix)
    original = [c['band_power'][:] for c in m['channels']]
    powers = copy.deepcopy(original)
    # Keep measured coherence at baseline. Candidate predictions are explicitly
    # approximate: changed cross terms and pre-compressor EQ require actual DSP.
    residual = [mix[b] - sum(p[b] for p in powers) for b in range(len(mix))]
    coherent = [abs(residual[b]) > max(mix[b], 1e-20) * .25 for b in range(len(mix))]
    def total(pp):
        return [max(0., residual[b] + sum(p[b] for p in pp)) for b in range(len(mix))]
    def score(pp):
        loss = objective(total(pp), target, edges)
        # Penalize losing an instrument's useful energy in bands that were not
        # excessive. This makes a local cut preferable to lowering a whole source
        # when both reduce the same excess.
        for row,base in zip(pp,original):
            norm = max(sum(base),1e-24)
            for b,(value,old) in enumerate(zip(row,base)):
                if target[b] is not None and mix[b] <= target[b] * 10**(.3) and old > 1e-20:
                    loss += 2. * old/norm * min(0.,db(max(value,1e-24)/old))**2
        return loss
    current = total(powers)
    initial_score = objective(current, target, edges)
    moves, cuts, faders = [], {}, {}
    for _ in range(max_steps):
        best = None
        old_score = score(powers)
        for i, c in enumerate(s['channels']):
            info = m['channels'][i]
            role = info.get('family', c['role'])
            if info['active_windows'] < 8 or role in ('drum_room', 'vocal_room', 'other'):
                continue
            # Preserve vocal body/articulation and fundamental regions, and avoid
            # EQ assumptions through substantial nonlinear compression.
            choices = []
            if faders.get(i, 0.) > -3. and c['fader_db'] + faders.get(i, 0.) >= -79.5:
                factors = [10 ** (-.5/10)] * len(mix)
                if all(not protected(role,a,z) or
                       db(max(1e-24,powers[i][j]*f)/max(1e-24,original[i][j])) >= -1.-1e-9
                       for j,(a,z,f) in enumerate(zip(edges,edges[1:],factors))):
                    choices.append(('fader', None, factors))
            if (info['max_reduction_p90_db'] or 0.) <= 3. and not c.get('dynamic_eq') and len(c['eq']) < 8:
                for b, (lo, hi) in enumerate(zip(edges, edges[1:])):
                    if target[b] is None or coherent[b]:
                        continue
                    if db(current[b]) - db(target[b]) <= 3. or powers[i][b] <= .1 * current[b]:
                        continue
                    if (cuts.get((i, b), 0.) <= -3.
                            or ((i, b) not in cuts and
                                (sum(ci == i for ci, _ in cuts) >= 2 or
                                 len(c['eq']) + sum(ci == i for ci, _ in cuts) >= 8))):
                        continue
                    hz = math.sqrt(lo * hi)
                    old_cut = cuts.get((i,b),0.)
                    factors = [bell_power(hz,.8,old_cut-.5,max(1.,math.sqrt(max(a,1.)*z)),s['sample_rate']) /
                               bell_power(hz,.8,old_cut,max(1.,math.sqrt(max(a,1.)*z)),s['sample_rate'])
                               for a,z in zip(edges,edges[1:])]
                    # Broad EQ skirts must also preserve each protected band.
                    if any(protected(role, a,z) and (db(f) < -.55 or
                           db(max(1e-24, powers[i][j] * f) / max(1e-24, original[i][j])) < -1.-1e-9)
                           for j,(a,z,f) in enumerate(zip(edges,edges[1:],factors))):
                        continue
                    choices.append(('eq', b, factors))
            for kind, band, factors in choices:
                trial_row = [a*f for a,f in zip(powers[i], factors)]
                if any(coherent[b] and abs(trial_row[b]-powers[i][b]) > max(mix[b],1e-20)*.01 for b in range(len(mix))):
                    continue
                trial = powers[:]
                trial[i] = trial_row
                spectrum = total(trial)
                trial_score = score(trial)
                # Do not buy an improvement by deepening a different deficient band.
                if any(t is not None and db(v) < db(t)-6. and db(v) < db(old)-.25 for v,old,t in zip(spectrum,current,target)):
                    continue
                improvement = old_score - trial_score
                if improvement > .01 and (best is None or improvement > best[0]):
                    best = (improvement, i, kind, band, trial_row, spectrum)
        if best is None:
            break
        improvement, i, kind, band, row, spectrum = best
        explanatory_band = band if band is not None else max(
            (b for b,t in enumerate(target) if t is not None),
            key=lambda b: max(0., db(current[b])-db(target[b])-3.) * powers[i][b]/max(current[b],1e-24))
        evidence = {'dominant_band_hz': edges[explanatory_band:explanatory_band+2],
                    'predicted_excess_before_db': db(current[explanatory_band])-db(target[explanatory_band]),
                    'source_to_mix_power_ratio': powers[i][explanatory_band]/max(current[explanatory_band],1e-24)}
        current = spectrum
        powers[i] = row
        if kind == 'fader':
            faders[i] = faders.get(i, 0.) - .5
        else:
            cuts[i, band] = cuts.get((i,band), 0.) - .5
        moves.append({'channel': i, 'file': s['channels'][i]['file'], 'control': kind,
                      'band_hz': None if band is None else edges[band:band+2],
                      'delta_db': -.5, 'objective_improvement': improvement, 'evidence': evidence})
    for i, delta in faders.items():
        s['channels'][i]['fader_db'] += delta
    for (i,b), delta in cuts.items():
        s['channels'][i]['eq'].append({'hz': math.sqrt(edges[b]*edges[b+1]), 'q': .8, 'db': delta})
    return {'version': 1, 'settings': s, 'moves': moves,
            'target_band_power': target, 'before_band_power': mix,
            'predicted_band_power': current, 'objective_before': initial_score,
            'objective_predicted': objective(current,target,edges),
            'objective_with_preservation_penalty': score(powers),
            'measurement_frozen_id': m.get('frozen_id'),
            'predicted_channel_band_power': powers,
            'stopping_reason': 'step budget reached' if len(moves)==max_steps else 'no further admissible improvement',
            'remaining_excess_bands': [dict(hz=edges[b:b+2], excess_db=db(current[b])-db(t))
                                      for b,t in enumerate(target) if t is not None and db(current[b])-db(t)>3.],
            'coherence_guard_bands': [edges[b:b+2] for b,flag in enumerate(coherent) if flag],
            'policy': 'Equal power per octave from 25 Hz to 12 kHz, octave-weighted median fitted level, 3 dB excess deadband. Subtractive, bounded 0.5 dB trials; protect vocals/fundamentals, reject unsupported/coherence-sensitive or heavily compressed EQ trials. No reference songs.',
            'limitations': 'Spectral shape objective, not a universal ideal or validated audibility model. Predicted curves use band-centre EQ response and fixed baseline coherence; actual compressor/FX/master interactions are not predicted. Existing baseline levels remain where no supported spectral correction exists. No independent backing-level solver or live controller is claimed.'}


def report_html(measurement, calculation, path):
    edges = measurement['edges_hz']
    maximum = min(16000., measurement['sample_rate']/2.)
    def plot(series):
        def x(hz):
            return 55+700*math.log(max(hz,25.)/25.)/math.log(maximum/25.)
        def y(level):
            return 225-200*(max(-90.,min(0.,level))+90.)/90.
        svg=['<svg viewBox="0 0 800 270" role="img" aria-label="Spectral density in dB per octave">']
        for level in (-80,-60,-40,-20,0):
            svg.append(f'<path d="M55 {y(level)}H755" stroke="#344155"/><text x="8" y="{y(level)+4}" fill="#aab">{level}</text>')
        for hz in (25,50,100,250,500,1000,2000,5000,10000):
            if hz<=maximum:
                svg.append(f'<text x="{x(hz):.1f}" y="248" fill="#aab" text-anchor="middle">{hz}</text>')
        for name,values,color in series:
            points=[]
            for lo,hi,value in zip(edges,edges[1:],values):
                if lo<25 or hi>maximum or value is None:
                    continue
                density=db(value/math.log2(hi/lo))
                points.append(f'{x(math.sqrt(lo*hi)):.1f},{y(density):.1f}')
            svg.append(f'<polyline points="{" ".join(points)}" fill="none" stroke="{color}" stroke-width="2.5"><title>{html.escape(name)}</title></polyline>')
        svg.append('</svg>')
        return ''.join(svg)
    sections=[]
    for i,c in enumerate(measurement['channels']):
        sections.append('<section><h2>'+html.escape(c['file'])+' · '+html.escape(c.get('family',c['role']))+'</h2>'
                        +f'<p>Active windows {c["active_windows"]}/{c["windows"]}; fader {c["fader_db"]:.2f} dB. '
                        +'Grey: raw capture. Blue: processed channel. Amber: predicted candidate.</p>'
                        +plot([('Raw',c['raw_band_power'],'#94a3b8'),('Processed',c['band_power'],'#60a5fa'),('Predicted',calculation['predicted_channel_band_power'][i],'#fbbf24')])+'</section>')
    moves=''.join('<tr><td>'+html.escape(move['file'])+'</td><td>'+move['control']+'</td><td>'+html.escape(str(move['band_hz']))+'</td><td>−0.5 dB</td></tr>' for move in calculation['moves'])
    main=plot([('Measured mix',calculation['before_band_power'],'#60a5fa'),
               ('Calculated prediction',calculation['predicted_band_power'],'#fbbf24'),
               ('Pink-spectrum hypothesis',calculation['target_band_power'],'#e879f9')])
    text='<!doctype html><meta charset="utf-8"><title>Calculated spectral balance</title><style>body{font:16px system-ui;background:#111827;color:#eee;max-width:1000px;margin:40px auto;padding:20px}section{border-top:1px solid #456;padding:12px}svg{width:100%}td{padding:8px}p{line-height:1.5}</style><h1>Measured channel spectra and calculated changes</h1><p>Production source DSP → channel spectra → coherent mix spectrum → explicit target curve → bounded source cuts → prepared settings.</p><p>'+html.escape(measurement['scope'])+'</p><p>'+html.escape(calculation['policy'])+'</p><h2>Mix and target · dB per octave</h2><p>Blue: measured. Amber: predicted candidate. Violet: target hypothesis, not a fault colour. Frequency in Hz.</p>'+main+'<p>'+html.escape(calculation['limitations'])+'</p><h2>Calculated moves</h2><table>'+moves+'</table>'+''.join(sections)
    Path(path).write_text(text)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest='command',required=True)
    a=sub.add_parser('compact');a.add_argument('analysis',type=Path);a.add_argument('output',type=Path);a.add_argument('--families',type=Path)
    a=sub.add_parser('calculate');a.add_argument('measurement',type=Path);a.add_argument('output',type=Path)
    args=p.parse_args()
    if args.command=='compact':
        compact(args.analysis,args.output, None if args.families is None else json.loads(args.families.read_text()))
    else:
        m=json.loads(args.measurement.read_text());r=calculate(m)
        args.output.mkdir(parents=True,exist_ok=False)
        write_json(args.output/'settings.json',r['settings'])
        write_json(args.output/'calculation.json',{k:v for k,v in r.items() if k!='settings'})
        report_html(m,r,args.output/'spectra.html')


if __name__=='__main__':
    main()
