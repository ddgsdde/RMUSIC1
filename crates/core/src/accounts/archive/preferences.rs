//! Portable music preferences retain unresolved catalogue references for export.
use super::*;
#[derive(Clone,Deserialize,Serialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
pub enum MusicPreference {
 Version{entity_id:String,track_id:Option<String>,revision:u64},
 Policy{entity_id:String,policy:crate::music_playback_policy::Policy,revision:u64},
}
impl MusicPreference {
 pub fn kind(&self)->&str{match self{Self::Version{..}=>"version",Self::Policy{..}=>"policy"}}
 pub fn entity(&self)->&str{match self{Self::Version{entity_id,..}|Self::Policy{entity_id,..}=>entity_id}}
 pub fn revision(&self)->u64{match self{Self::Version{revision,..}|Self::Policy{revision,..}=>*revision}}
 pub fn cursor(&self)->String{format!("{}\x1f{}",self.kind(),self.entity())}
 pub fn valid(&self)->bool{let valid=|v:&str,max:usize|!v.is_empty()&&v.len()<=max&&!v.chars().any(char::is_control);valid(self.entity(),200)&&self.revision()<i64::MAX as u64&&match self{Self::Version{track_id,..}=>track_id.as_ref().is_none_or(|id|valid(id,256)),Self::Policy{policy,..}=>policy.preferred_track_id.as_ref().is_none_or(|id|valid(id,256))}}
 pub fn apply(&self,c:&rusqlite::Connection,user:&str)->anyhow::Result<()>{
  let raw=serde_json::to_string(self)?;
  c.execute("INSERT OR IGNORE INTO account_music_preferences(user_id,kind,entity_id,data) VALUES(?1,?2,?3,?4)",params![user,self.kind(),self.entity(),raw])?;
  // Existing destination choices win in a merge. A missing recording or file
  // remains portable metadata; it is never guessed from a song title.
  if !c.query_row("SELECT EXISTS(SELECT 1 FROM music_entities WHERE id=?1)",[self.entity()],|r|r.get::<_,bool>(0))?{return Ok(());}
  match self{
   Self::Version{entity_id,track_id,revision}=>{if track_id.as_ref().is_none_or(|id|c.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[id],|r|r.get::<_,bool>(0)).unwrap_or(false)){c.execute("INSERT OR IGNORE INTO music_version_preferences(user_id,entity_id,track_id,revision) VALUES(?1,?2,?3,?4)",params![user,entity_id,track_id,*revision as i64])?;}},
   Self::Policy{entity_id,policy,revision}=>{c.execute("INSERT OR IGNORE INTO music_playback_policies(user_id,entity_id,revision,data) VALUES(?1,?2,?3,?4)",params![user,entity_id,*revision as i64,serde_json::to_string(policy)?])?;},
  }Ok(())
 }
}
pub fn export(c:&rusqlite::Connection,user:&str,cursor:&str)->anyhow::Result<Vec<Record>>{
 let(kind,key)=cursor.split_once('\x1f').unwrap_or(("",""));
 let sql="SELECT kind,entity_id,data FROM (SELECT 'version' kind,entity_id,json_object('type','version','entity_id',entity_id,'track_id',track_id,'revision',revision) data FROM music_version_preferences WHERE user_id=?1 AND ('version',entity_id)>(?2,?3) UNION ALL SELECT 'policy',entity_id,json_object('type','policy','entity_id',entity_id,'revision',revision,'policy',json(data)) FROM music_playback_policies WHERE user_id=?1 AND ('policy',entity_id)>(?2,?3) UNION ALL SELECT kind,entity_id,data FROM account_music_preferences p WHERE user_id=?1 AND (kind,entity_id)>(?2,?3) AND NOT EXISTS(SELECT 1 FROM music_version_preferences v WHERE p.kind='version' AND v.user_id=p.user_id AND v.entity_id=p.entity_id) AND NOT EXISTS(SELECT 1 FROM music_playback_policies v WHERE p.kind='policy' AND v.user_id=p.user_id AND v.entity_id=p.entity_id)) ORDER BY kind,entity_id LIMIT ?4";
 let rows=c.prepare(sql)?.query_map(params![user,kind,key,BATCH as i64],|r|r.get::<_,String>(2))?.collect::<rusqlite::Result<Vec<_>>>()?;
 rows.into_iter().map(|raw|Ok(Record::MusicPreference{data:serde_json::from_str(&raw)?})).collect()
}
