//! The coordinator sees synthetic device receipts; this does not open a DAC.
use super::*;

#[tokio::test]
async fn group_output_actual_source_position_is_independent_of_the_maximum_heard_time() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;
    f.s.db.run(|c|{c.execute("UPDATE tracks SET duration=120.0",[])?;Ok(())}).await.unwrap();
    let mut req=request(&track,&p);req["queue"]=json!([track]);req["position"]=json!(10.);
    let(commands,_,_)=begin(&f,&mut p,&req,768000).await;
    for (i,frames,position) in [(0,384000,10.5003),(1,383999,10.5004)] {
        let receipt=json!({"type":"state","zone_id":p[i].zone,"core_playback_id":commands[i]["playback_id"],"track_id":track,"status":"playing","position":position,"frames":frames,"sample_rate":768000});
        report(&f,&p[i],receipt).await;
    }
    assert_eq!(total(&f).await,0.5);assert_eq!(sessions(&f).await,1);
    let position=f.s.db.run(|c|Ok(c.query_row("SELECT last_position FROM play_sessions WHERE zone_id LIKE 'sync:%'",[],|r|r.get::<_,f64>(0))?)).await.unwrap();
    assert_eq!(position,10.5004,"logical source position must not be reconstructed from output seconds or taken only from the longest output");
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let current=f.call("GET",&path,None).await.1;assert_eq!(current["listened_seconds"],0.5);
    let mut bad=json!({"type":"state","zone_id":p[1].zone,"core_playback_id":commands[1]["playback_id"],"track_id":track,"status":"playing","position":10.5,"frames":384001,"sample_rate":768000});
    report(&f,&p[1],bad.clone()).await;assert_eq!(total(&f).await,0.5,"backwards source position cannot mutate the group");
    bad["position"]=json!(10.501);report(&f,&p[1],bad.clone()).await;report(&f,&p[1],bad).await;assert!((total(&f).await-384001./768000.).abs()<1e-12);
}
