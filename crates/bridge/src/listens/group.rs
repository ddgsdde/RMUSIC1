//! Immutable native group finals, isolated from personal listening and recovery cursors.
use super::*;
use serde_json::{json,Value};
use sha2::{Digest,Sha256};

pub(super) fn create(db:&Connection)->Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS group_output_finals(id TEXT PRIMARY KEY,core_id TEXT NOT NULL,user_id TEXT NOT NULL,epoch_id TEXT NOT NULL,group_id TEXT NOT NULL,payload TEXT NOT NULL,ack INTEGER NOT NULL DEFAULT 0,error TEXT,UNIQUE(core_id,user_id,epoch_id)); CREATE INDEX IF NOT EXISTS group_final_owner ON group_output_finals(core_id,user_id,ack);")?;Ok(())
}
#[derive(Clone)]
pub(crate) struct Recorder {store:Arc<Store>,owner:Owner,endpoint:String}
impl Recorder {
    pub(crate) fn new(store:Arc<Store>,owner:Owner,endpoint:String)->Self {Self{store,owner,endpoint}}
    // Called only after worker join, or for a ticket that never started a worker.
    pub(crate) fn capture(&self,report:&Value)->Result<()> {
        if report["type"]!="start_state"||report["measured_version"]!=2||report["final"]!=true {anyhow::bail!("not a final group-v2 receipt");}
        let group=report["start"]["plan_id"].as_str().context("group final has no plan")?;uuid::Uuid::parse_str(group)?;
        let epoch:String=Sha256::digest(serde_json::to_vec(&json!([self.endpoint,report["start"]]))?).iter().map(|b|format!("{b:02x}")).collect();
        let host=rmusic_audio::output::clock::host_ns().context("group final host clock unavailable")?;
        let at=chrono::Utc::now().timestamp_millis();let mut db=self.store.db.lock().map_err(|_|anyhow::anyhow!("group final journal lock poisoned"))?;let tx=db.transaction()?;
        let prior:Option<String>=tx.query_row("SELECT payload FROM group_output_finals WHERE core_id=?1 AND user_id=?2 AND epoch_id=?3",params![self.owner.core_id,self.owner.user_id,epoch],|r|r.get(0)).optional()?;
        if let Some(prior)=prior {let mut repeated:Value=serde_json::from_str(&prior)?;repeated["report"]=report.clone();if repeated.to_string()!=prior{anyhow::bail!("immutable group final changed");}return Ok(());}
        let id=uuid::Uuid::new_v4().to_string();let body=json!({"request_id":id,"endpoint_id":self.endpoint,"captured_at":at,"captured_host_ns":host,"report":report});
        tx.execute("INSERT INTO group_output_finals(id,core_id,user_id,epoch_id,group_id,payload) VALUES(?1,?2,?3,?4,?5,?6)",params![id,self.owner.core_id,self.owner.user_id,epoch,group,body.to_string()])?;tx.commit()?;Ok(())
    }
}
#[derive(Clone)]
pub struct Row {pub id:String,pub group_id:String,pub body:Value,payload:String}
pub fn pending(store:&Store,owner:&Owner)->Result<Vec<Row>> {
    let db=store.db.lock().map_err(|_|anyhow::anyhow!("group final journal lock poisoned"))?;let mut q=db.prepare("SELECT id,group_id,payload FROM group_output_finals WHERE core_id=?1 AND user_id=?2 AND ack=0 AND error IS NULL ORDER BY rowid LIMIT 32")?;
    let rows=q.query_map(params![owner.core_id,owner.user_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|(id,group_id,body)|Ok(Row{id,group_id,body:serde_json::from_str(&body)?,payload:body})).collect()
}
fn acknowledge(store:&Store,owner:&Owner,id:&str,error:Option<&str>)->Result<()> {
    store.db.lock().map_err(|_|anyhow::anyhow!("group final journal lock poisoned"))?.execute("UPDATE group_output_finals SET ack=?4,error=?5 WHERE id=?1 AND core_id=?2 AND user_id=?3",params![id,owner.core_id,owner.user_id,error.is_none(),error])?;Ok(())
}
pub(super) async fn upload(store:Arc<Store>,owner:Owner,config:&crate::sequence::Config,status:&Value,rows:Vec<Row>)->Result<usize> {
    if status["core_id"]!=owner.core_id {anyhow::bail!("group final upload core identity mismatch");}
    if status["capabilities"]["group_output_final_v2"]!=true{return Ok(0);}
    let mut accepted=0;
    for row in rows {
        let response=config.upload_group_final(&row.group_id,&row.payload).await?;let code=response.status();
        if code==reqwest::StatusCode::CONFLICT {continue;} // A live/settling epoch will be eligible later.
        let error=if matches!(code.as_u16(),400|404|422){Some(format!("http_{}",code.as_u16()))}else{
            #[derive(Deserialize)] struct Ack {applied:bool}
            let ack=response.error_for_status()?.json::<Ack>().await?;let _=ack.applied;None
        };
        let db=store.clone();let selected=owner.clone();let id=row.id;
        if error.is_some(){eprintln!("Group final rejected ({code}); immutable evidence retained locally");}else{accepted+=1;}
        tokio::task::spawn_blocking(move||acknowledge(&db,&selected,&id,error.as_deref())).await??;
    }Ok(accepted)
}

#[cfg(test)]
#[path="group_tests.rs"]
mod tests;
