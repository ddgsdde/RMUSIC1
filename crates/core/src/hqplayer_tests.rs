use super::*;
async fn endpoint(f:&Fixture)->tokio::sync::mpsc::Receiver<Value>{
    f.s.endpoints.write().await.insert("hqplayer-test".into(),json!({"id":"hqplayer-test","name":"HQPlayer","kind":"hqplayer","online":true,"verified":false,"capabilities":{"dsp":false,"transitions":false}}));
    let(tx,rx)=tokio::sync::mpsc::channel(64);f.s.bridges.write().await.insert("hqplayer-test".into(),tx);rx
}
#[tokio::test]
async fn hqplayer_availability_requires_an_online_registered_endpoint(){
    let f=Fixture::new().await;assert_eq!(f.call("GET","/api/v1/settings",None).await.1["hqplayer_available"],false);
    let _rx=endpoint(&f).await;assert_eq!(f.call("GET","/api/v1/settings",None).await.1["hqplayer_available"],true);
    f.s.endpoints.write().await.get_mut("hqplayer-test").unwrap()["online"]=json!(false);
    assert_eq!(f.call("GET","/api/v1/settings",None).await.1["hqplayer_available"],false);
}
#[tokio::test]
async fn unsupported_hqplayer_processing_is_rejected_before_saving_or_dispatch(){
    let f=Fixture::new().await;let mut rx=endpoint(&f).await;
    assert_eq!(f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"endpoint_id":"hqplayer-test"}))).await.0,StatusCode::OK);
    while rx.try_recv().is_ok(){}
    let before=f.call("GET","/api/v1/zones/zone-local",None).await.1;
    for(method,path,body)in [("PUT","/api/v1/zones/zone-local/dsp",json!({"enabled":true})),("PATCH","/api/v1/zones/zone-local",json!({"transitions":{"fade_in_ms":500,"fade_out_ms":0}}))]{
        assert_eq!(f.call(method,path,Some(body)).await.0,StatusCode::BAD_REQUEST);
        assert_eq!(f.call("GET","/api/v1/zones/zone-local",None).await.1,before);assert!(rx.try_recv().is_err());
    }
}
#[tokio::test]
async fn switching_to_hqplayer_requires_disabling_existing_processing_first(){
    let f=Fixture::new().await;let _rx=endpoint(&f).await;
    f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":true}))).await;
    let before=f.call("GET","/api/v1/zones/zone-local",None).await.1;
    assert_eq!(f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"endpoint_id":"hqplayer-test"}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("GET","/api/v1/zones/zone-local",None).await.1,before);
    f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":false}))).await;
    assert_eq!(f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"endpoint_id":"hqplayer-test"}))).await.0,StatusCode::OK);
}
