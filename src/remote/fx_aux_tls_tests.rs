//! Actual Brain palette over authenticated loopback QUIC; synthetic source only.
use super::*;
use crate::remote::fx_aux_owner as owner;
use sha2::Digest;
use std::{cell::Cell, rc::Rc};
async fn reply(c: &mut RemoteClient) -> Value {
    match c.receive().await.unwrap() {
        Response::Reply { payload, .. } => payload,
        r => panic!("{r:?}"),
    }
}
fn read() -> Value {
    json!({"contract":"GP-FX-aux","version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}})
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "GP_FX_LIBRARY actual palette; bounded private loopback TLS/source/Brain wet acceptance"]
async fn actual_four_aux_tls_brain_controls_and_samples() {
    four_aux_tls_episode(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GP_FX_LIBRARY and GP_FX_DESK_TEST_EXE; disarmed saved FX authenticated loopback acceptance"]
async fn actual_saved_fx_tls_frontend_and_silence() {
    four_aux_tls_episode(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GP_FX_LIBRARY and GP_FX_DESK_TEST_EXE; actual Master authenticated loopback and wet samples"]
async fn actual_master_tls_frontend_and_samples() {
    four_aux_tls_episode(false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GP_FX_LIBRARY and GP_FX_DESK_TEST_EXE; disarmed autonomous Master authenticated loopback acceptance"]
async fn actual_master_tls_frontend_disarmed_silence() {
    four_aux_tls_episode(true, true).await;
}
async fn four_aux_tls_episode(saved: bool, master: bool) {
    tokio::task::LocalSet::new().run_until(async{tokio::time::timeout(Duration::from_secs(15),async{
  let directory=PrivateTestDirectory::new();let library=std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());let hash=crate::hash::hex(sha2::Sha256::digest(std::fs::read(&library).unwrap()));
  let mut provider=crate::local_audio::LocalAudio::bind_configured(&directory.0,"host",SHOW,Counter(9),EngineTopology::software(4,2,0).unwrap()).unwrap();if !saved {provider.rearm().unwrap();}let mut host=Box::new(HostAuthority::new(provider,1).unwrap());let identity=host.identity();
  let core_before=serde_json::to_value(host.provider_mut().persisted_intent().unwrap()).unwrap();
  let clock_status=if saved {Some(host.start_source_clock(0,Duration::from_millis(1),20000).unwrap())}else{None};
  let driver = std::env::var_os("GP_FX_DESK_TEST_EXE");
  let driver_done = Rc::new(Cell::new(false)); let source_done = driver_done.clone(); let with_driver = driver.is_some();
  let(server_credentials,client_credentials)=credentials();let policy=PolicyStore::new(vec![peer(client_credentials.certificate_chain[0].as_ref(),&if master {vec![Permission::Fx,Permission::FxConfiguration,Permission::Foh]} else {vec![Permission::Fx,Permission::FxConfiguration]})]).unwrap();let client_policy=PolicyStore::new(vec![peer(server_credentials.certificate_chain[0].as_ref(),&[])]).unwrap();
  let server=RemoteServer::bind("127.0.0.1:0".parse().unwrap(),&server_credentials,policy).unwrap();let address=server.local_addr().unwrap();
  if driver.is_some() {
    use std::os::unix::fs::PermissionsExt;
    let cert=directory.0.join("desk.der"); let key=directory.0.join("desk.key"); let ca=directory.0.join("ca.der");
    std::fs::write(&cert,client_credentials.certificate_chain[0].as_ref()).unwrap();
    std::fs::write(&key,client_credentials.private_key.secret_der()).unwrap(); std::fs::set_permissions(&key,std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&ca,client_credentials.trust_roots[0].as_ref()).unwrap();
    let config=json!({"show":SHOW,"saved_directory":directory.0,"remote":{"bind":"127.0.0.1:0","server":address.to_string(),"server_name":"stagebox.test","certificate":cert,"private_key":key,"ca":ca,"server_certificate_sha256":fingerprint(server_credentials.certificate_chain[0].as_ref()),"peer_id":"brain"}});
    std::fs::write(directory.0.join("desk.json"),serde_json::to_vec(&config).unwrap()).unwrap();
  }
  let(tx,mut rx)=tokio::sync::mpsc::channel(2);
  let acceptor=tokio::spawn(async move{loop{let session=server.accept().await.unwrap();let(mut proxy,mailbox)=authority_channel(session.context(),identity.clone()).unwrap();tx.send(mailbox).await.unwrap();tokio::spawn(async move{if let Err(e)=session.run(&mut proxy).await{eprintln!("aux TLS server: {e}");}});}});
  let stop=Rc::new(Cell::new(false));let stop_source=stop.clone();let wet_count=Rc::new(Cell::new(0u64));let wet_source=wet_count.clone();let stalled=Rc::new(Cell::new(false));let stall_brain=stalled.clone();let source=tokio::task::spawn_local(async move{
   let mut mailboxes=Vec::new();let mut timer=tokio::time::interval(Duration::from_millis(1));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);let capture:[f64;192]=std::array::from_fn(|i|((i%4+1)*128)as f64/8388608.);let mut output=[0.;192];let mut comparisons=0;let mut monitors=None;
   while !stop_source.get(){timer.tick().await;while let Ok(m)=rx.try_recv(){mailboxes.push(m);}let now=monotonic_ms();for m in &mut mailboxes{m.service(host.as_mut(),now).unwrap();}let frame=host.provider().frame();if saved {host.service_source_clock(now).unwrap();} else {host.process_source(now,9,frame,&capture,&mut output).unwrap();}wet_source.set(host.media_stats().accepted_wet_packets);
    for f in 0..48 {
      if frame>=480 && monitors.is_none() { monitors=Some([output[f*4+2],output[f*4+3]]); }
      if !saved && (!with_driver || source_done.get()) {
        for ch in 0..2 { let expected=host.provider().brain_fx_send()[f*2+ch]+host.test_admitted_wet()[f*2+ch]*if with_driver && !master {0.95}else{1.}; assert!((output[f*4+ch]-expected * if master {10_f64.powf(-12. / 20.)} else {1.}).abs()<1e-12); comparisons+=1; }
        if let Some(baseline)=monitors { let wet=if with_driver && !master {(host.test_admitted_wet()[f*2]+host.test_admitted_wet()[f*2+1])*0.025}else{0.}; for m in 0..2 { assert!((output[f*4+2+m]-baseline[m]-wet*(m+1) as f64).abs()<1e-12); } }
      }
    }
   }
   if let Some(status)=clock_status {
       let report=host.finish_source_clock(&status).unwrap();
       assert!(report.fault.is_none(),"{:?}",report.fault);
       assert!(report.peaks.iter().all(|v|*v==0.) && report.energy.iter().all(|v|*v==0.));
       let mut expected_core = core_before;
       if master { for p in expected_core["engine"]["parameters"].as_array_mut().unwrap() {
           if p["target"]["parameter"] == "master_gain" { p["value"] = json!(-12000); }
       } }
       assert_eq!(serde_json::to_value(host.provider_mut().persisted_intent().unwrap()).unwrap(),expected_core);
       comparisons=report.blocks as usize*48*4;
       eprintln!("saved FX TLS: {comparisons} silent output samples; core/composed intent unchanged");
   }
   (host,comparisons)
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut brain=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy.clone()).await.unwrap();let ids=owner::channel_ids();let max=brain.max_datagram();let mut streams=grouped_streams(MediaRole::FxSend,&ids,48,0,1,max).unwrap();streams.extend(grouped_streams(MediaRole::WetReturn,&ids,48,768,streams.len()as u32+1,max).unwrap());let identity=brain.identity().clone();let d=MediaDescriptor{session:Counter(brain.session()),source_epoch:identity.source_epoch,capability_generation:identity.capability_generation,map_generation:identity.map_generation,sample_rate:48000,streams};brain.negotiate(d).await.unwrap();assert!(matches!(brain.receive().await.unwrap(),Response::MediaAccepted{..}));
  let mut worker=BrainWorker::prepare_aux(brain.media_channel().unwrap(),&library,&hash,4).unwrap();worker.start_aux_execution(std::time::Instant::now()+Duration::from_secs(15)).unwrap();let(mut controls,reader)=brain.take_fx_replies().unwrap();let stop_brain=stop.clone();let brain_task=tokio::task::spawn_local(async move{
   let mut timer=tokio::time::interval(Duration::from_millis(10));let mut energy=[0.;4];while !stop_brain.get(){if stall_brain.get(){tokio::time::sleep(Duration::from_millis(1)).await;continue;}tokio::select!{
    r=worker.step()=>{r.unwrap();for frame in worker.test_last_wet().chunks_exact(8){for b in 0..4{energy[b]+=frame[b*2].powi(2)+frame[b*2+1].powi(2);}}},
    r=controls.recv()=>match r.unwrap().unwrap(){Response::Reply{payload,..}=>if let Some(c)=payload.get("command") && let Some(message)=worker.aux_command(serde_json::from_value(c.clone()).unwrap()).unwrap(){brain.send_command(json!({"contract":owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":brain.writer(),"message":message})).await.unwrap();},r=>panic!("{r:?}")},
    _=timer.tick()=>if let Some(message)=worker.poll_aux_observation().unwrap(){brain.send_command(json!({"contract":owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":brain.writer(),"message":message})).await.unwrap();}
   }}reader.abort();worker.finish_execution();brain.close();(worker.stats.processed_blocks,worker.test_execution_energy().unwrap_or(energy),worker.execution_report())
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut desk=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy).await.unwrap();eprintln!("aux TLS: desk attached; reading authority");desk.send_command(snapshot_request()).await.unwrap();let raw=reply(&mut desk).await;let revision=raw["snapshot"]["authority"]["revision"].clone();desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":null,"request_id":"1","expected_revision":revision,"kind":"grant","body":{"scope":"fx_configuration"}})).await.unwrap();let grant=reply(&mut desk).await;eprintln!("aux TLS: grant received");let lease=grant["outcome"]["body"]["granted_lease"].clone();assert!(lease.is_string(),"{grant}");
  if !saved {
  for(bus,algorithm)in [1,6,8,10].into_iter().enumerate(){eprintln!("aux TLS: bus {bus} read/review");
   let mut snapshot=loop{desk.send_command(read()).await.unwrap();let s=reply(&mut desk).await;if s["available"]==true&&s["frame"]!="0"{break s;}tokio::time::sleep(Duration::from_millis(10)).await;};
   desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":lease,"request_id":(bus*2+2).to_string(),"expected_revision":snapshot["revision"],"kind":"renew","body":{}})).await.unwrap();let renewed=reply(&mut desk).await;assert_ne!(renewed["outcome"]["kind"],"rejected","{renewed}");desk.send_command(read()).await.unwrap();snapshot=reply(&mut desk).await;
   let palette:Value=serde_json::from_str(snapshot["palette_json"].as_str().unwrap()).unwrap();let defaults=palette.as_array().unwrap().iter().find(|p|p["algorithm"]==algorithm).unwrap()["defaults"].clone();let mut c:Value=serde_json::from_str(snapshot["buses"][bus]["configuration_json"].as_str().unwrap()).unwrap();c["settings"]=defaults;if matches!(algorithm,1|8){c["settings"]["time_ms"]=json!(2.);}c["sends"][bus]=json!(1.);
   eprintln!("aux TLS: bus {bus} configuring");let body=json!({"bus":bus,"map_revision":snapshot["map_revision"],"library_sha256":snapshot["library_sha256"],"owner_binding":snapshot["owner_binding"],"expected_generation":snapshot["buses"][bus]["generation"],"configuration_json":c.to_string(),"panic":false});desk.send_command(json!({"contract":"GP-FX-aux","version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":lease,"request_id":(bus*2+3).to_string(),"expected_revision":snapshot["revision"],"kind":"fx_aux_configure","body":body})).await.unwrap();loop{let r=reply(&mut desk).await;eprintln!("aux TLS: bus {bus} state {}",r["state"]);if !["preparing","prepared","accepted"].contains(&r["state"].as_str().unwrap_or("")){stop.set(true);desk.close();let(_,_,execution)=brain_task.await.unwrap();let(mut host,_)=source.await.unwrap();host.finish_source_egress();acceptor.abort();panic!("{r}; execution={execution:?}; media={}; egress={}",serde_json::to_value(host.media_stats()).unwrap(),host.source_egress_report());}
  if r["state"]=="accepted"{break;}}
  }
  }
  desk.send_command(read()).await.unwrap();let final_snapshot=reply(&mut desk).await;if let Some(path)=std::env::var_os("GP_FX_AUX_REMOTE_FIXTURE"){std::fs::write(path,serde_json::to_vec_pretty(&final_snapshot).unwrap()).unwrap();}
  if let Some(exe)=driver {
    desk.close();
    tokio::time::sleep(Duration::from_millis(20)).await;
    struct Child(std::process::Child);
    impl Drop for Child { fn drop(&mut self) { let _=self.0.kill(); let _=self.0.wait(); } }
    let names = if master {vec!["actual_provider_master_controls"]} else {vec![if saved {"actual_provider_saved_fx_controls"} else {"actual_provider_return_controls"}]};
    for name in names {
    let mut child=Child(std::process::Command::new(&exe).args(["--ignored","--exact",name,"--nocapture"])
      .env("GP_FX_DESK_CONFIG",directory.0.join("desk.json"))
      .spawn().unwrap());
    loop { if let Some(status)=child.0.try_wait().unwrap() { if !status.success() { stop.set(true); let (_,_,execution)=brain_task.await.unwrap(); let (mut host,_)=source.await.unwrap(); host.finish_source_egress(); acceptor.abort(); panic!("Desk driver failed; execution={execution:?}; media={}; egress={}",serde_json::to_value(host.media_stats()).unwrap(),host.source_egress_report()); } break; } tokio::time::sleep(Duration::from_millis(5)).await; }
    }
    driver_done.set(true);
  }
  let before=wet_count.get();stalled.set(true);tokio::time::sleep(Duration::from_millis(100)).await;let during=wet_count.get();assert!(during>before+20,"wet return stopped with controller: {before}->{during}");stalled.set(false);
  tokio::time::sleep(Duration::from_millis(80)).await;stop.set(true);desk.close();let(blocks,energy,execution)=brain_task.await.unwrap();let execution=execution.unwrap();assert_eq!(execution["input_drops"],0,"{execution}");assert_eq!(execution["output_drops"],0,"{execution}");assert!(execution["first_source_gap"].is_null(),"{execution}");let(host,comparisons)=source.await.unwrap();acceptor.abort();assert!(blocks>0&&comparisons>0&&(saved || energy.iter().all(|v|*v>0.)),"{energy:?}");if !saved {assert!(host.media_stats().nonzero_rendered_wet_samples>0);}eprintln!("actual TLS four-aux blocks {blocks}, Main sample comparisons {comparisons}, wet energies {energy:?}");if saved {eprintln!("saved FX TLS transport: media={}; egress={}",serde_json::to_value(host.media_stats()).unwrap(),host.source_egress_report());}drop(host);
 }).await.expect("bounded four-aux TLS acceptance");}).await;
}
