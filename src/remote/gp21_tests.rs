//! Explicit actual-owner TLS/media acceptance; no physical endpoints.
use super::*;
use crate::fx_wire::{self, Configuration, Mutation};
use sha2::Digest;
use std::{cell::Cell, rc::Rc};
async fn reply(client: &mut RemoteClient) -> Value {
    match client.receive().await.unwrap() {
        Response::Reply { payload, .. } => payload,
        other => panic!("unexpected {other:?}"),
    }
}

fn fx_read() -> Value {
    json!({"contract":fx_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_snapshot","body":{}})
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires GP21_FX_LIBRARY, GP21_MODULE_MANIFEST and GP21_PA_CONFIGURATION actual hash-pinned software owners"]
async fn gp21_actual_tls_brain_boundary_wet_pa_and_raw_recording() {
    tokio::task::LocalSet::new().run_until(async {
 tokio::time::timeout(Duration::from_secs(12),async {
  use std::os::unix::fs::PermissionsExt;
  let path=std::env::temp_dir().join(format!("gp21-tls-{}",std::process::id()));std::fs::create_dir(&path).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o700)).unwrap();
  let library=std::path::PathBuf::from(std::env::var_os("GP21_FX_LIBRARY").unwrap());
  let hash=crate::hash::hex(sha2::Sha256::digest(std::fs::read(&library).unwrap()));
  let mut provider=crate::local_audio::LocalAudio::bind_configured(&path,"host",SHOW,Counter(9),EngineTopology::software(16,4,2).unwrap()).unwrap();
  provider.enable_modules(std::path::Path::new(&std::env::var_os("GP21_MODULE_MANIFEST").unwrap())).unwrap();
  provider.configure_pa(&std::fs::read(std::env::var_os("GP21_PA_CONFIGURATION").unwrap()).unwrap()).unwrap();
  provider.rearm().unwrap();super::super::runner::start_recording(&mut provider,SHOW,"fx-witness",monotonic_ms()).unwrap();
  let m=crate::module_graph::Manifest::load(std::path::Path::new(&std::env::var_os("GP21_MODULE_MANIFEST").unwrap())).unwrap();
  let mut pa_reference=crate::host::pa_v2::Pa::load(&m.pa.library,&std::fs::read(std::env::var_os("GP21_PA_CONFIGURATION").unwrap()).unwrap(),9,0).unwrap();assert_eq!(pa_reference.rearm(9,0),0);
  let mut host=HostAuthority::new(provider,1).unwrap();let ident=host.identity();
  let (server_credentials,client_credentials)=credentials();
  let policy=PolicyStore::new(vec![peer(client_credentials.certificate_chain[0].as_ref(),&[Permission::Fx,Permission::FxConfiguration])]).unwrap();
  let client_policy=PolicyStore::new(vec![peer(server_credentials.certificate_chain[0].as_ref(),&[])]).unwrap();
  let server=RemoteServer::bind("127.0.0.1:0".parse().unwrap(),&server_credentials,policy).unwrap();let address=server.local_addr().unwrap();
  let (tx,mut rx)=tokio::sync::mpsc::channel(2);
  let acceptor=tokio::spawn(async move {loop {let session=server.accept().await.unwrap();let(mut proxy,mailbox)=authority_channel(session.context(),ident.clone()).unwrap();tx.send(mailbox).await.unwrap();tokio::spawn(async move{let _=session.run(&mut proxy).await;});}});
  let stop=Rc::new(Cell::new(false));let stop_host=stop.clone();
  let source=tokio::task::spawn_local(async move {
   let mut mailboxes=Vec::new();let mut timer=tokio::time::interval(Duration::from_millis(1));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
   let capture:Vec<f64>=(0..48*16).map(|i|((i%16+1)*128) as f64/8388608.).collect();let mut output=vec![0.;48*host.provider().topology().playback_channels];let mut output_nonzero=0usize;let mut pa_compared=0usize;
   while !stop_host.get() {timer.tick().await;while let Ok(m)=rx.try_recv(){mailboxes.push(m);}let now=monotonic_ms();for m in &mut mailboxes {m.service(&mut host,now).unwrap();}let frame=host.provider().frame();host.process_source(now,9,frame,&capture,&mut output).unwrap();
    let program:Vec<f64>=host.provider().brain_fx_send().iter().zip(host.test_admitted_wet()).map(|(dry,wet)|dry+wet).collect();let mut expected=[0.;96];assert_eq!(pa_reference.process(&program,&mut expected,9,frame),0);
    let width=host.provider().topology().playback_channels;
    for port in &host.provider().topology().outputs {if let Some(crate::topology::OutputSource::Pa{index})=port.source {for f in 0..48 {assert_eq!(output[f*width+port.playback_slot],expected[f*2+index],"PA after actual dry+wet frame{frame}");pa_compared+=1;}}}
    output_nonzero+=output.iter().filter(|s|**s!=0.).count();}
   host.provider_mut().quiesce_source("gp21_software_end").unwrap();
   for _ in 0..500 {let s=host.provider_mut().module_status(monotonic_ms()).unwrap();if s["recording"]["state"]=="finalized" {break;}tokio::time::sleep(Duration::from_millis(1)).await;}
   (host,output_nonzero,pa_compared)
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut brain=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy.clone()).await.unwrap();
  let mut d=descriptor(&context(&[Permission::Fx]),16);d.session=Counter(brain.session());d.source_epoch=brain.identity().source_epoch;d.capability_generation=brain.identity().capability_generation;d.map_generation=brain.identity().map_generation;
  d.streams.retain(|s|s.role!=MediaRole::Analysis);
  brain.negotiate(d).await.unwrap();assert!(matches!(brain.receive().await.unwrap(),Response::MediaAccepted{..}));
  let mut worker=BrainWorker::prepare(brain.media_channel().unwrap(),&library).unwrap();assert!(worker.enable_fx_control(&library,&hash).unwrap());
  let (mut controls,reader)=brain.take_fx_replies().unwrap();let stop_brain=stop.clone();
  let brain_task=tokio::task::spawn_local(async move {
   let mut timer=tokio::time::interval(Duration::from_millis(10));let mut selected_zero=0usize;let mut unchanged_nonzero=0usize;
   let initial=worker.fx_observation().unwrap();
   let early=Mutation{binding:initial.binding,expected_generation:initial.generation,expected_reset_count:initial.reset_count,lead_frames:Counter(4800),configuration_json:Some(initial.owner_json),panic_mask:None};
   assert!(matches!(worker.fx_command(fx_wire::OwnerCommand::Prepare{ticket:Counter(999),mutation:early,apply_frame:Counter(4800)}).unwrap(),Some(fx_wire::OwnerMessage::Refused{..})));

   while !stop_brain.get() {
    tokio::select! {
     r=worker.step()=>{r.unwrap();let o=worker.fx_observation().unwrap();if o.generation.0==1 && o.remaining_frames==[0,0] && o.next_source_frame.0>=o.settled_source_frame.0+48 {for pair in worker.test_last_wet().chunks_exact(2) {assert_eq!(pair[0],0.,"reviewed streamed left must be exact zero after settlement");selected_zero+=1;unchanged_nonzero+=usize::from(pair[1]!=0.);}}},
     r=controls.recv()=>match r.unwrap().unwrap() {
      Response::Reply{payload,..}=>if let Some(c)=payload.get("command") {let command=serde_json::from_value(c.clone()).unwrap();if let Some(message)=worker.fx_command(command).unwrap(){brain.send_command(json!({"contract":fx_wire::CONTRACT,"version":1,"kind":"fx_owner","writer":brain.writer(),"message":message})).await.unwrap();}},
      Response::Refused{reason,..}=>panic!("owner refused {reason}"),other=>panic!("{other:?}")
     },
     _=timer.tick()=>if let Some(message)=worker.poll_fx_observation().unwrap(){brain.send_command(json!({"contract":fx_wire::CONTRACT,"version":1,"kind":"fx_owner","writer":brain.writer(),"message":message})).await.unwrap();}
    }
   }
   reader.abort();brain.close();(worker.stats.processed_blocks,worker.nonzero_return_samples(),selected_zero,unchanged_nonzero)
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut desk=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy).await.unwrap();
  desk.send_command(snapshot_request()).await.unwrap();let audio=reply(&mut desk).await;let revision=audio["snapshot"]["revision"].clone();
  // Engine snapshots expose revision in their authority snapshot in some versions.
  let revision=if revision.is_null(){audio["snapshot"]["authority"]["revision"].clone()}else{revision};
  desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":null,"request_id":"1","expected_revision":revision,"kind":"grant","body":{"scope":"fx_configuration"}})).await.unwrap();let grant=reply(&mut desk).await;let lease=grant["outcome"]["body"]["granted_lease"].clone();assert!(lease.is_string(),"{grant}");
  let snapshot=loop {desk.send_command(fx_read()).await.unwrap();let s=reply(&mut desk).await;if s["available"]==true && s["observation"]["next_source_frame"]!="0" {break s;}tokio::time::sleep(Duration::from_millis(10)).await;};
  let o:fx_wire::Observation=serde_json::from_value(snapshot["observation"].clone()).unwrap();let mut config=Configuration::decode(&o.owner_json).unwrap();config.channels[0].delay_ms=1.;config.channels[0].feedback=0.;config.channels[0].damping=0.;config.channels[0].wet_gain=0.;
  let body=Mutation{binding:o.binding,expected_generation:o.generation,expected_reset_count:o.reset_count,lead_frames:Counter(4800),configuration_json:Some(serde_json::to_string(&config).unwrap()),panic_mask:None};
  desk.send_command(json!({"contract":fx_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":lease,"request_id":"2","expected_revision":snapshot["revision"],"kind":"fx_configure","body":body})).await.unwrap();
  let mut states=Vec::new();let settled=loop {let r=reply(&mut desk).await;let state=r["state"].as_str().unwrap_or("invalid");states.push(state.to_owned());assert!(!["refused","cancelled","unknown","authorization_replayed"].contains(&state),"{r}");if state=="settled"{break r;}};
  let applied:u64=settled["observation"]["applied_source_frame"].as_str().unwrap().parse().unwrap();let end:u64=settled["observation"]["settled_source_frame"].as_str().unwrap().parse().unwrap();assert_eq!(end-applied,960);assert!(states.iter().any(|s|s=="permitted"));
  tokio::time::sleep(Duration::from_millis(30)).await;stop.set(true);desk.close();let (blocks,wet,selected_zero,unchanged_nonzero)=brain_task.await.unwrap();let (mut host,output,pa_compared)=source.await.unwrap();assert!(selected_zero>0 && unchanged_nonzero>0 && pa_compared>0);acceptor.abort();assert!(blocks>0 && wet>0 && output>0);assert!(host.media_stats().nonzero_rendered_wet_samples>0);
  let modules=host.provider_mut().module_status(monotonic_ms()).unwrap();assert_eq!(modules["recording"]["state"],"finalized","{modules}");
  let take=path.join("takes/fx-witness");let mut stems:Vec<_>=std::fs::read_dir(&take).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|s|s=="wav")).collect();stems.sort();assert_eq!(stems.len(),16);
  for (channel,stem) in stems.iter().enumerate(){let mut wav=hound::WavReader::open(stem).unwrap();let samples:Vec<i32>=wav.samples::<i32>().map(|s|s.unwrap()).collect();assert!(!samples.is_empty());assert!(samples.iter().all(|s|*s==((channel+1)*128) as i32),"raw channel {channel}");}
  if let Some(out)=std::env::var_os("GP21_TLS_EVIDENCE"){std::fs::write(out,serde_json::to_vec_pretty(&json!({"states":states,"settled":settled,"actual_owner_blocks":blocks,"actual_owner_nonzero_wet":wet,"provider_nonzero_wet":host.media_stats().nonzero_rendered_wet_samples,"nonzero_pa_output":output,"raw_recording_tracks":stems.len(),"raw_exact":true,"selected_left_exact_zero_samples":selected_zero,"unchanged_right_nonzero_samples":unchanged_nonzero,"pa_reference_equal_samples":pa_compared})).unwrap()).unwrap();}
  drop(host);std::fs::remove_dir_all(path).unwrap();
 }).await.expect("bounded actual GP21 episode");
 }).await;
}
