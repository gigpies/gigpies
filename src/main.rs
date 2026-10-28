use std::{ffi::OsStr, path::Path, process::ExitCode};

fn run() -> gigpies::inventory::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => println!(
            "GigPies {} — experimental offline automixer\nRun gigpies --help for commands.",
            env!("CARGO_PKG_VERSION")
        ),
        [arg] if arg == "--help" || arg == "-h" => println!(
            "GigPies — experimental offline automixer\n\nUsage:\n  gigpies inspect <WAV-or-directory>\n  gigpies eq-maps\n  gigpies eq-map-check <map-id-or-path>\n  gigpies eq-map-import <baseline.json> <source-dir> <new-map.json> <import-spec.json>\n  gigpies eq-match-compare <reference.json> <changed.json> <source-dir> <new-output-dir> <comparison-request.json>\n  gigpies eq-match-compare-review <comparison.json> <new-summary.md>\n  gigpies eq-match-plan <baseline.json> <source-dir> <new-output-dir> <request.json>\n  gigpies eq-match-amount <state.json> <source-dir> <new-output-dir> <map-id> <0-100>\n  gigpies eq-match-apply <state.json> <source-dir> <new-output-dir> <selection.json>\n  gigpies eq-match-reset <state.json> <new-output-dir>\n  gigpies preset <new-settings.json>\n  gigpies soundcheck <settings.json> <source-dir> <new-output-dir> <finish-seconds>\n  gigpies render <prepared.json> <source-dir> <new-output-dir>\n  gigpies delivery-policy <new-policy.json>\n  gigpies render-policy <prepared.json> <source-dir> <new-output-dir> <policy.json>\n  gigpies delivery-finalize <render-dir> <source-dir> <new-output-dir> <policy.json>\n  gigpies delivery-check <delivery-dir> <source-dir>\n  gigpies peak-measure <stereo.wav> <new-report.json>\n  gigpies observe-stages <prepared.json> <source-dir> <new-output-dir> <end-seconds>\n  gigpies fx-preset <prepared.json> <new-fx-settings.json>\n  gigpies ambience-plan <prepared.json> <source-dir> <new-output-dir> <fx-policy.json>\n  gigpies ambience-check <plan-dir> <source-dir>\n  gigpies ambience-audit <plan-dir> <new-audit-dir>\n  gigpies finish <prepared-fx.json> <source-dir> <new-output-dir> [policy.json]\n  gigpies preserve-source <initial-source-settings.json> <new-output-dir>\n  gigpies unity-pass <settings.json> <source-dir> <new-output-dir>\n  gigpies loudness-policy <new-policy.json>\n  gigpies loudness-pass <settings.json> <source-dir> <new-output-dir> <policy.json>\n  gigpies balance-source-analyze <initial-source-settings.json> <source-dir> <new-output-dir>\n  gigpies balance-analyze <settings.json> <source-dir> <new-output-dir> [policy.json]\n  gigpies balance-pass <settings.json> <source-dir> <new-output-dir> [policy.json]\n  gigpies tone-pass <settings.json> <source-dir> <new-output-dir> <tone-policy.json>\n  gigpies tone-analyze <settings.json> <source-dir> <new-output-dir> <tone-policy.json>\n  gigpies ensemble-request <new-request.json>\n  gigpies ensemble-analyze <baseline.json> <source-dir> <new-output-dir> <request.json>\n  gigpies ensemble-prepare <state.json> <source-dir> <new-output-dir>\n  gigpies ensemble-listen-pack <state.json> <source-dir> <candidate-dir> <new-output-dir>\n  gigpies ensemble-reset <state.json> <new-output-dir>\n  gigpies source-pass <settings.json> <source-dir> <new-output-dir> <source-policy.json>\n  gigpies source-analyze <settings.json> <source-dir> <new-output-dir> <source-policy.json>\n  gigpies reference-review <ours.wav> <reference.wav> <new-output-dir> [excerpt-start-seconds]\n  gigpies snare-reference-fit <source-dir> <pilot-dir> <new-model.json> <start-seconds> <end-seconds>\n  gigpies snare-reference-evaluate <source-dir> <review-dir> <model.json> <new-audit.json>\n  gigpies snare-predict <source-dir> <pilot-dir> <new-predictions.json> <start-seconds> <end-seconds>\n  gigpies snare-bleed-verify <source-dir> <pilot-dir> <new-output-dir> <start-seconds> <end-seconds>\n  gigpies snare-bleed <baseline.json> <source-dir> <new-output-dir> <bleed-policy.json> <start-seconds> <end-seconds>\n  gigpies drum-events <measurement.json> <new-diagnosis.json>\n  gigpies drum-verify <baseline.json> <candidate.json> <source-dir> <new-output-dir> <drum-policy.json> <start-seconds> <end-seconds>\n  gigpies drum-correct <prepared.json> <source-dir> <new-output-dir> <drum-policy.json> <start-seconds> <end-seconds>\n  gigpies drum-analyze <prepared.json> <source-dir> <new-output-dir> <drum-policy.json> <start-seconds> <end-seconds>\n  gigpies bass-analyze <prepared.json> <source-dir> <new-output-dir> <bass-policy.json> <start-seconds> <end-seconds>\n  gigpies bass-correct <prepared.json> <source-dir> <new-output-dir> <bass-policy.json> <start-seconds> <end-seconds>\n  gigpies compare <old.wav> <new.wav> <new-output-dir>\n  gigpies --version\n\ninspect prints JSON WAV header metadata; directories are nonrecursive.\nsoundcheck and render write local audio without playback or hardware access."
        ),
        [arg] if arg == "--version" || arg == "-V" => {
            println!("gigpies {}", env!("CARGO_PKG_VERSION"))
        }
        [command, path] if command == OsStr::new("inspect") => {
            let report = gigpies::inventory::inspect(Path::new(path))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        [command, path] if command == "preset" => {
            gigpies::automix::write_json(Path::new(path), &gigpies::automix::config::example())?;
        }
        [command] if command == "eq-maps" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&gigpies::automix::matching::builtin_maps())?
            );
        }
        [command, map] if command == "eq-map-check" => {
            let m = gigpies::automix::matching::load_map(map.to_str().ok_or("invalid map path")?)?;
            println!("{} v{}: valid {:?}", m.id, m.version, m.kind);
        }
        [command, config, root, out, spec] if command == "eq-map-import" => {
            use gigpies::automix::matching as m;
            m::import(
                m::read_json(Path::new(config))?,
                Path::new(root),
                Path::new(out),
                m::read_json(Path::new(spec))?,
            )?;
        }
        [command, reference, changed, root, out, request] if command == "eq-match-compare" => {
            use gigpies::automix::matching as m;
            m::compare_eq(
                m::read_json(Path::new(reference))?,
                m::read_json(Path::new(changed))?,
                Path::new(root),
                Path::new(out),
                m::read_json(Path::new(request))?,
            )?;
            println!(
                "Paired EQ evidence saved. Diagnostic review: {}",
                Path::new(out).join("COMPARISON.md").display()
            );
        }
        [command, report, output] if command == "eq-match-compare-review" => {
            gigpies::automix::matching::review_comparison(Path::new(report), Path::new(output))?;
            println!("Saved diagnostic summary: {}", Path::new(output).display());
        }
        [command, config, root, out, request] if command == "eq-match-plan" => {
            use gigpies::automix::matching as m;
            let state = m::plan(
                m::read_json(Path::new(config))?,
                Path::new(root),
                Path::new(out),
                m::read_json(Path::new(request))?,
            )?;
            for c in state.choices {
                println!("{}: {}", c.map.id, c.proposal.reason);
            }
            println!("Frozen review saved at 0%; select an amount to create a preview.");
        }
        [command, state, root, out, selection] if command == "eq-match-apply" => {
            use gigpies::automix::matching as m;
            let state = m::select(
                m::read_json(Path::new(state))?,
                Path::new(root),
                Path::new(out),
                m::read_json(Path::new(selection))?,
            )?;
            println!(
                "Applied amount: {}%; inspect state.json for checks.",
                state.amount_percent
            );
        }
        [command, state, root, out, map, amount] if command == "eq-match-amount" => {
            use gigpies::automix::matching as m;
            let state: m::State = m::read_json(Path::new(state))?;
            let map_id = map.to_str().ok_or("invalid map id")?.to_string();
            let selection = m::Selection {
                frozen_id: state.frozen_id.clone(),
                map_version: state.choice(&map_id)?.map.version,
                map_id,
                amount_percent: amount.to_str().ok_or("invalid amount")?.parse()?,
            };
            let state = m::select(state, Path::new(root), Path::new(out), selection)?;
            println!(
                "Applied amount: {}%; inspect state.json for checks.",
                state.amount_percent
            );
        }
        [command, state, out] if command == "eq-match-reset" => {
            use gigpies::automix::matching as m;
            m::reset(m::read_json(Path::new(state))?, Path::new(out))?;
        }
        [command, input, output] if command == "preserve-source" => {
            let session = serde_json::from_reader(std::fs::File::open(input)?)?;
            gigpies::automix::preservation::prepare(session, Path::new(output))?;
        }
        [command, input, output] if command == "fx-preset" => {
            let mut session = serde_json::from_reader(std::fs::File::open(input)?)?;
            gigpies::automix::effects::add_pass(&mut session)?;
            gigpies::automix::write_json(Path::new(output), &session)?;
        }
        [command, config, root, out, policy] if command == "ambience-plan" => {
            let review = gigpies::automix::ambience::run(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
            )?;
            println!(
                "{} Review: {}",
                review.summary(),
                Path::new(out).join("REVIEW.md").display()
            );
        }
        [command, plan, root] if command == "ambience-check" => {
            let review = gigpies::automix::ambience::verify(Path::new(plan), Path::new(root))?;
            println!(
                "Saved settings, reports and source identities verified. {} Review: {}",
                review.summary(),
                Path::new(plan).join("REVIEW.md").display()
            );
        }
        [command, plan, out] if command == "ambience-audit" => {
            let audit = gigpies::automix::ambience::audit_saved(Path::new(plan), Path::new(out))?;
            println!(
                "Saved evidence audited: {} decay minimums, {} bounded returns. Review: {}",
                audit.minimum_decay_limits(),
                audit.return_limits(),
                Path::new(out).join("AUDIT.md").display()
            );
        }
        [command, config, root, out, seconds] if command == "soundcheck" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let seconds = seconds.to_str().ok_or("invalid seconds")?.parse()?;
            gigpies::automix::run(session, Path::new(root), Path::new(out), Some(seconds))?;
        }
        [command, config, root, out, request] if command == "ensemble-analyze" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let request = serde_json::from_reader(std::fs::File::open(request)?)?;
            gigpies::automix::ensemble::workflow::analyze(
                session,
                Path::new(root),
                Path::new(out),
                request,
            )?;
        }
        [command, state, root, out] if command == "ensemble-prepare" => {
            let state = serde_json::from_reader(std::fs::File::open(state)?)?;
            gigpies::automix::ensemble::workflow::prepare(state, Path::new(root), Path::new(out))?;
        }
        [command, state, root, candidate, out] if command == "ensemble-listen-pack" => {
            gigpies::automix::ensemble::listening::prepare(
                gigpies::automix::matching::read_json(Path::new(state))?,
                Path::new(root),
                Path::new(candidate),
                Path::new(out),
            )?;
            println!(
                "Saved randomized original-level excerpts; listener judgment remains pending."
            );
        }
        [command, state, out] if command == "ensemble-reset" => {
            let state = serde_json::from_reader(std::fs::File::open(state)?)?;
            gigpies::automix::ensemble::workflow::reset(state, Path::new(out))?;
        }
        [command, out] if command == "ensemble-request" => {
            gigpies::automix::write_json(
                Path::new(out),
                &gigpies::automix::ensemble::Request::default(),
            )?;
        }
        [command, config, root, out, policy]
            if command == "source-pass" || command == "source-analyze" =>
        {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let policy = serde_json::from_reader(std::fs::File::open(policy)?)?;
            gigpies::automix::expert::run(
                session,
                Path::new(root),
                Path::new(out),
                policy,
                command == "source-pass",
            )?;
        }
        [command, config, root, out, policy]
            if command == "tone-pass" || command == "tone-analyze" =>
        {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let policy = serde_json::from_reader(std::fs::File::open(policy)?)?;
            gigpies::automix::tone::run(
                session,
                Path::new(root),
                Path::new(out),
                policy,
                command == "tone-pass",
            )?;
        }
        [command, config, root, out, policy] if command == "finish" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let policy = serde_json::from_reader(std::fs::File::open(policy)?)?;
            gigpies::automix::analysis::finish(session, Path::new(root), Path::new(out), policy)?;
        }
        [command, out] if command == "loudness-policy" => {
            gigpies::automix::write_json(
                Path::new(out),
                &gigpies::automix::hierarchy::Policy::default(),
            )?;
        }
        [command, config, root, out, policy] if command == "loudness-pass" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let policy = serde_json::from_reader(std::fs::File::open(policy)?)?;
            gigpies::automix::hierarchy::run(session, Path::new(root), Path::new(out), policy)?;
        }
        [command, config, root, out, policy]
            if command == "balance-pass" || command == "balance-analyze" =>
        {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            let policy = serde_json::from_reader(std::fs::File::open(policy)?)?;
            gigpies::automix::balance::run(
                session,
                Path::new(root),
                Path::new(out),
                Some(policy),
                command == "balance-pass",
            )?;
        }
        [command, config, root, out]
            if command == "balance-pass" || command == "balance-analyze" =>
        {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            gigpies::automix::balance::run(
                session,
                Path::new(root),
                Path::new(out),
                None,
                command == "balance-pass",
            )?;
        }
        [command, config, root, out] if command == "balance-source-analyze" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            gigpies::automix::balance::source_analyze(session, Path::new(root), Path::new(out))?;
        }
        [command, config, root, out] if command == "unity-pass" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            gigpies::automix::unity::run(session, Path::new(root), Path::new(out))?;
        }
        [command, config, root, out] if command == "finish" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            gigpies::automix::analysis::finish(
                session,
                Path::new(root),
                Path::new(out),
                Default::default(),
            )?;
        }
        [command, ours, reference, out] if command == "reference-review" => {
            gigpies::automix::reference::run(
                Path::new(ours),
                Path::new(reference),
                Path::new(out),
                None,
            )?;
        }
        [command, ours, reference, out, start] if command == "reference-review" => {
            gigpies::automix::reference::run(
                Path::new(ours),
                Path::new(reference),
                Path::new(out),
                Some(start.to_str().ok_or("invalid start")?.parse()?),
            )?;
        }
        [command, root, pilot, out, start, end] if command == "snare-reference-fit" => {
            gigpies::automix::bleed_reference::fit_saved(
                Path::new(root),
                Path::new(pilot),
                Path::new(out),
                [
                    start.to_str().ok_or("invalid start")?.parse()?,
                    end.to_str().ok_or("invalid end")?.parse()?,
                ],
            )?;
        }
        [command, root, review, frozen, out] if command == "snare-reference-evaluate" => {
            gigpies::automix::bleed_reference::evaluate_saved(
                Path::new(root),
                Path::new(review),
                Path::new(frozen),
                Path::new(out),
            )?;
        }
        [command, root, pilot, out, start, end]
            if command == "snare-predict" || command == "snare-bleed-verify" =>
        {
            let start = start.to_str().ok_or("invalid start")?.parse()?;
            let end = end.to_str().ok_or("invalid end")?.parse()?;
            if command == "snare-predict" {
                gigpies::automix::bleed::predict_saved(
                    Path::new(root),
                    Path::new(pilot),
                    Path::new(out),
                    start,
                    end,
                )?;
            } else {
                gigpies::automix::bleed::inspect_frozen(
                    Path::new(root),
                    Path::new(pilot),
                    Path::new(out),
                    start,
                    end,
                )?;
            }
        }
        [command, config, root, out, policy, start, end] if command == "snare-bleed" => {
            gigpies::automix::bleed::run(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, input, out] if command == "drum-events" => {
            gigpies::automix::drums::redetect(Path::new(input), Path::new(out))?;
        }
        [command, config, candidate, root, out, policy, start, end] if command == "drum-verify" => {
            gigpies::automix::drums::verify(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                serde_json::from_reader(std::fs::File::open(candidate)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, config, root, out, policy, start, end] if command == "drum-correct" => {
            gigpies::automix::drums::correct(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, config, root, out, policy, start, end] if command == "drum-analyze" => {
            gigpies::automix::drums::analyze(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, config, root, out, policy, start, end] if command == "bass-correct" => {
            gigpies::automix::bass::run(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, config, root, out, policy, start, end] if command == "bass-analyze" => {
            gigpies::automix::bass::analyze(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
                start.to_str().ok_or("invalid start")?.parse()?,
                end.to_str().ok_or("invalid end")?.parse()?,
            )?;
        }
        [command, old, new, out] if command == "compare" => {
            gigpies::automix::compare(Path::new(old), Path::new(new), Path::new(out))?;
        }
        [command, out] if command == "delivery-policy" => {
            gigpies::automix::write_json(
                Path::new(out),
                &gigpies::automix::delivery::Policy::default(),
            )?;
        }
        [command, input, out] if command == "peak-measure" => {
            gigpies::automix::write_json(
                Path::new(out),
                &gigpies::automix::true_peak::measure(Path::new(input))?,
            )?;
        }
        [command, config, root, out, end] if command == "observe-stages" => {
            gigpies::automix::observe(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                end.to_str().ok_or("invalid observation end")?.parse()?,
            )?;
        }
        [command, config, root, out, policy] if command == "render-policy" => {
            gigpies::automix::run_policy(
                serde_json::from_reader(std::fs::File::open(config)?)?,
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
            )?;
        }
        [command, render, root, out, policy] if command == "delivery-finalize" => {
            gigpies::automix::delivery::finalize(
                Path::new(render),
                Path::new(root),
                Path::new(out),
                serde_json::from_reader(std::fs::File::open(policy)?)?,
            )?;
        }
        [command, out, root] if command == "delivery-check" => {
            gigpies::automix::delivery::check(Path::new(out), Path::new(root))?;
        }
        [command, config, root, out] if command == "render" => {
            let session = serde_json::from_reader(std::fs::File::open(config)?)?;
            gigpies::automix::run(session, Path::new(root), Path::new(out), None)?;
        }
        _ => return Err("unknown command or arguments; run gigpies --help".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("gigpies: {error}");
            ExitCode::FAILURE
        }
    }
}
