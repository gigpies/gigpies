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
    tokio::task::LocalSet::new().run_until(async{tokio::time::timeout(Duration::from_secs(15),async{
  let directory=PrivateTestDirectory::new();let library=std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());let hash=format!("{:x}",sha2::Sha256::digest(std::fs::read(&library).unwrap()));
  let mut provider=crate::local_audio::LocalAudio::bind_configured(&directory.0,"host",SHOW,Counter(9),EngineTopology::software(4,2,0).unwrap()).unwrap();provider.rearm().unwrap();let mut host=Box::new(HostAuthority::new(provider,1).unwrap());let identity=host.identity();
  let(server_credentials,client_credentials)=credentials();let policy=PolicyStore::new(vec![peer(client_credentials.certificate_chain[0].as_ref(),&[Permission::Fx,Permission::FxConfiguration])]).unwrap();let client_policy=PolicyStore::new(vec![peer(server_credentials.certificate_chain[0].as_ref(),&[])]).unwrap();
  let server=RemoteServer::bind("127.0.0.1:0".parse().unwrap(),&server_credentials,policy).unwrap();let address=server.local_addr().unwrap();let(tx,mut rx)=tokio::sync::mpsc::channel(2);
  let acceptor=tokio::spawn(async move{loop{let session=server.accept().await.unwrap();let(mut proxy,mailbox)=authority_channel(session.context(),identity.clone()).unwrap();tx.send(mailbox).await.unwrap();tokio::spawn(async move{if let Err(e)=session.run(&mut proxy).await{eprintln!("aux TLS server: {e}");}});}});
  let stop=Rc::new(Cell::new(false));let stop_source=stop.clone();let source=tokio::task::spawn_local(async move{
   let mut mailboxes=Vec::new();let mut timer=tokio::time::interval(Duration::from_millis(1));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);let capture:[f64;192]=std::array::from_fn(|i|((i%4+1)*128)as f64/8388608.);let mut output=[0.;192];let mut comparisons=0;let mut monitors=None;
   while !stop_source.get(){timer.tick().await;while let Ok(m)=rx.try_recv(){mailboxes.push(m);}let now=monotonic_ms();for m in &mut mailboxes{m.service(host.as_mut(),now).unwrap();}let frame=host.provider().frame();host.process_source(now,9,frame,&capture,&mut output).unwrap();
    for f in 0..48{for ch in 0..2{let expected=host.provider().brain_fx_send()[f*2+ch]+host.test_admitted_wet()[f*2+ch];assert!((output[f*4+ch]-expected).abs()<1e-12);comparisons+=1;}if frame>=480{let baseline=monitors.get_or_insert([output[f*4+2],output[f*4+3]]);assert_eq!([output[f*4+2],output[f*4+3]],*baseline);}}
   }(host,comparisons)
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut brain=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy.clone()).await.unwrap();let ids=owner::channel_ids();let max=brain.max_datagram();let mut streams=grouped_streams(MediaRole::FxSend,&ids,48,0,1,max).unwrap();streams.extend(grouped_streams(MediaRole::WetReturn,&ids,48,960,streams.len()as u32+1,max).unwrap());let identity=brain.identity().clone();let d=MediaDescriptor{session:Counter(brain.session()),source_epoch:identity.source_epoch,capability_generation:identity.capability_generation,map_generation:identity.map_generation,sample_rate:48000,streams};brain.negotiate(d).await.unwrap();assert!(matches!(brain.receive().await.unwrap(),Response::MediaAccepted{..}));
  let mut worker=BrainWorker::prepare_aux(brain.media_channel().unwrap(),&library,&hash,4).unwrap();let(mut controls,reader)=brain.take_fx_replies().unwrap();let stop_brain=stop.clone();let brain_task=tokio::task::spawn_local(async move{
   let mut timer=tokio::time::interval(Duration::from_millis(10));let mut energy=[0.;4];while !stop_brain.get(){tokio::select!{
    r=worker.step()=>{r.unwrap();for frame in worker.test_last_wet().chunks_exact(8){for b in 0..4{energy[b]+=frame[b*2].powi(2)+frame[b*2+1].powi(2);}}},
    r=controls.recv()=>match r.unwrap().unwrap(){Response::Reply{payload,..}=>if let Some(c)=payload.get("command") && let Some(message)=worker.aux_command(serde_json::from_value(c.clone()).unwrap()).unwrap(){brain.send_command(json!({"contract":owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":brain.writer(),"message":message})).await.unwrap();},r=>panic!("{r:?}")},
    _=timer.tick()=>if let Some(message)=worker.poll_aux_observation().unwrap(){brain.send_command(json!({"contract":owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":brain.writer(),"message":message})).await.unwrap();}
   }}reader.abort();brain.close();(worker.stats.processed_blocks,energy)
  });
  let endpoint=client_endpoint("127.0.0.1:0".parse().unwrap(),&client_credentials).unwrap();let mut desk=RemoteClient::connect(endpoint,address,"stagebox.test",client_policy).await.unwrap();eprintln!("aux TLS: desk attached; reading authority");desk.send_command(snapshot_request()).await.unwrap();let raw=reply(&mut desk).await;let revision=raw["snapshot"]["authority"]["revision"].clone();desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":null,"request_id":"1","expected_revision":revision,"kind":"grant","body":{"scope":"fx_configuration"}})).await.unwrap();let grant=reply(&mut desk).await;eprintln!("aux TLS: grant received");let lease=grant["outcome"]["body"]["granted_lease"].clone();assert!(lease.is_string(),"{grant}");
  for(bus,algorithm)in [1,6,8,10].into_iter().enumerate(){eprintln!("aux TLS: bus {bus} read/review");
   let mut snapshot=loop{desk.send_command(read()).await.unwrap();let s=reply(&mut desk).await;if s["available"]==true&&s["frame"]!="0"{break s;}tokio::time::sleep(Duration::from_millis(10)).await;};
   desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":lease,"request_id":(bus*2+2).to_string(),"expected_revision":snapshot["revision"],"kind":"renew","body":{}})).await.unwrap();let renewed=reply(&mut desk).await;assert_ne!(renewed["outcome"]["kind"],"rejected","{renewed}");desk.send_command(read()).await.unwrap();snapshot=reply(&mut desk).await;
   let palette:Value=serde_json::from_str(snapshot["palette_json"].as_str().unwrap()).unwrap();let defaults=palette.as_array().unwrap().iter().find(|p|p["algorithm"]==algorithm).unwrap()["defaults"].clone();let mut c:Value=serde_json::from_str(snapshot["buses"][bus]["configuration_json"].as_str().unwrap()).unwrap();c["settings"]=defaults;if matches!(algorithm,1|8){c["settings"]["time_ms"]=json!(2.);}c["sends"][bus]=json!(1.);
   eprintln!("aux TLS: bus {bus} configuring");let body=json!({"bus":bus,"map_revision":snapshot["map_revision"],"library_sha256":snapshot["library_sha256"],"owner_binding":snapshot["owner_binding"],"expected_generation":snapshot["buses"][bus]["generation"],"configuration_json":c.to_string(),"panic":false});desk.send_command(json!({"contract":"GP-FX-aux","version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":desk.writer(),"lease":lease,"request_id":(bus*2+3).to_string(),"expected_revision":snapshot["revision"],"kind":"fx_aux_configure","body":body})).await.unwrap();loop{let r=reply(&mut desk).await;eprintln!("aux TLS: bus {bus} state {}",r["state"]);assert!(["preparing","prepared","accepted"].contains(&r["state"].as_str().unwrap_or("")),"{r}");if r["state"]=="accepted"{break;}}
  }
  desk.send_command(read()).await.unwrap();let final_snapshot=reply(&mut desk).await;if let Some(path)=std::env::var_os("GP_FX_AUX_REMOTE_FIXTURE"){std::fs::write(path,serde_json::to_vec_pretty(&final_snapshot).unwrap()).unwrap();}
  tokio::time::sleep(Duration::from_millis(80)).await;stop.set(true);desk.close();let(blocks,energy)=brain_task.await.unwrap();let(host,comparisons)=source.await.unwrap();acceptor.abort();assert!(blocks>0&&comparisons>0&&energy.iter().all(|v|*v>0.),"{energy:?}");assert!(host.media_stats().nonzero_rendered_wet_samples>0);eprintln!("actual TLS four-aux blocks {blocks}, Main sample comparisons {comparisons}, wet energies {energy:?}");drop(host);
 }).await.expect("bounded four-aux TLS acceptance");}).await;
}
