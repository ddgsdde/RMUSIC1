use super::*;
#[derive(Clone,Debug,Deserialize,Serialize)]#[serde(deny_unknown_fields)]pub struct Origin{pub core:String,pub user:String,pub id:String}
impl Origin{
 pub fn valid(&self)->bool{[&self.core,&self.user,&self.id].iter().all(|v|!v.is_empty()&&v.len()<=200&&!v.chars().any(char::is_control))}
 pub fn key(&self)->String{format!("{}\x1f{}\x1f{}",self.core,self.user,self.id)}
}
#[derive(Clone,Debug,Deserialize,Serialize)]#[serde(deny_unknown_fields)]pub struct History {
 pub session_id:String,pub track_id:String,pub title:String,pub artist:String,#[serde(default)]pub album:String,#[serde(default)]pub album_artist:String,
 pub cover_url:Option<String>,pub source:String,pub started_at_ms:i64,pub finished_at_ms:Option<i64>,pub finish_reason:Option<String>,pub listened_seconds:f64,pub counted:bool,pub duration:f64,#[serde(default)]pub imported:bool,
}
impl History{pub fn valid(&self)->bool{self.session_id.len()<=200&&text(&self.track_id,256)&&self.title.len()<=8192&&self.artist.len()<=8192&&self.album.len()<=8192&&self.album_artist.len()<=8192&&self.source.len()<=80&&time(self.started_at_ms)&&self.finished_at_ms.is_none_or(|t|time(t)&&t>=self.started_at_ms)&&finite(self.listened_seconds,31_536_000.)&&finite(self.duration,86400.)&&self.finish_reason.as_ref().is_none_or(|v|v.len()<=80)&&image(&self.cover_url)}}
#[derive(Clone,Debug,Deserialize,Serialize,PartialEq)]#[serde(deny_unknown_fields)]pub struct Stat{#[serde(default)]pub revision:u64,pub minute:i64,pub track_id:String,pub title:String,pub artist:String,pub cover_url:Option<String>,pub play_count:u64,pub skip_count:u64,pub play_seconds:f64,pub last_played_at:Option<i64>}
impl Stat{pub fn valid(&self)->bool{self.minute>=0&&self.minute<=4_223_371_679&&text(&self.track_id,256)&&self.title.len()<=8192&&self.artist.len()<=8192&&self.play_count<=1_000_000&&self.skip_count<=1_000_000&&finite(self.play_seconds,86400.)&&self.last_played_at.is_none_or(|v|(0..=253_402_300_799).contains(&v))&&image(&self.cover_url)}}
#[derive(Clone,Debug,Deserialize,Serialize)]#[serde(deny_unknown_fields)]pub struct HistoryEntry{pub origin:Origin,pub record:History}
#[derive(Clone,Debug,Deserialize,Serialize)]#[serde(deny_unknown_fields)]pub struct StatEntry{pub origin:Origin,pub record:Stat}
#[derive(Clone,Deserialize,Serialize)]#[serde(deny_unknown_fields)]pub struct Playlist{pub id:String,pub name:String,pub track_ids:Vec<String>,pub smart_query:Option<crate::smart_rules::Rule>,#[serde(default)]pub favorites:bool}
#[derive(Clone,Deserialize,Serialize)]#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]pub enum Record{
 Manifest{schema:u32,origin_core:String,origin_user:String,username:String},History{data:HistoryEntry},Stat{data:StatEntry},Playlist{data:Playlist},Preferences{data:Value},MusicPreference{data:super::preferences::MusicPreference},
}
impl Record {
 pub fn kind(&self)->&'static str{match self{Self::Manifest{..}=>"manifest",Self::History{..}=>"history",Self::Stat{..}=>"stat",Self::Playlist{..}=>"playlist",Self::Preferences{..}=>"preferences",Self::MusicPreference{..}=>"music_preference"}}
 pub fn cursor(&self)->String{match self{Self::Manifest{..}=>String::new(),Self::History{data}=>data.record.session_id.clone(),Self::Stat{data}=>data.origin.key(),Self::Playlist{data}=>data.id.clone(),Self::Preferences{..}=>"done".into(),Self::MusicPreference{data}=>data.cursor()}}
 pub fn valid(&self)->bool{match self {
  Self::Manifest{schema,origin_core,origin_user,username}=>*schema==1&&text(origin_core,200)&&text(origin_user,200)&&text(username,80),
  Self::History{data}=>data.origin.valid()&&data.record.valid(),Self::Stat{data}=>data.origin.valid()&&data.record.valid(),
  Self::Playlist{data}=>text(&data.id,200)&&text(&data.name,100)&&data.track_ids.len()<=10000&&data.track_ids.iter().all(|id|text(id,256))&&data.smart_query.as_ref().is_none_or(|v|v.validate().is_ok()),
  Self::MusicPreference{data}=>data.valid(),
  Self::Preferences{data}=>data.is_object()&&data.to_string().len()<=256*1024,
 }}
}
fn text(v:&str,max:usize)->bool{!v.trim().is_empty()&&v.len()<=max&&!v.chars().any(char::is_control)}
fn finite(v:f64,max:f64)->bool{v.is_finite()&&v>=0.&&v<=max}
fn time(t:i64)->bool{(0..=253_402_300_799_999).contains(&t)}
fn image(v:&Option<String>)->bool{v.as_ref().is_none_or(|v|v.len()<=4096&&(v.starts_with("/api/v1/")&&!v.contains('?')||url::Url::parse(v).is_ok_and(|u|matches!(u.scheme(),"https"|"http")&&u.username().is_empty()&&u.password().is_none()&&!u.query_pairs().any(|(k,_)|["ticket","token","key","password"].contains(&k.to_lowercase().as_str())))))}
pub fn safe_preferences(value:Value)->Value{
 fn walk(v:Value,depth:usize)->Value{if depth>8{return Value::Null;}match v{Value::Object(m)=>Value::Object(m.into_iter().filter(|(k,_)|{let k=k.to_lowercase();!["token","password","secret","cookie","credential","session","totp","certificate","core_url"].iter().any(|s|k.contains(s))}).map(|(k,v)|(k,walk(v,depth+1))).collect()),Value::Array(a)=>Value::Array(a.into_iter().take(10000).map(|v|walk(v,depth+1)).collect()),v=>v}}
 walk(value,0)
}

// Each imported contribution retains its source identity. Replaying an archive
// takes maxima for that contribution and credits only the new delta.
pub(crate) fn apply_record(c:&rusqlite::Connection,user:&str,core:&str,origin_core:&str,origin_user:&str,record:Record)->anyhow::Result<()> {
 match record {
  Record::History{data}=>{
   let HistoryEntry{origin,mut record}=data;
   let old=c.query_row("SELECT data FROM account_history WHERE user_id=?1 AND origin_core=?2 AND origin_user=?3 AND origin_id=?4",params![user,origin.core,origin.user,origin.id],|r|r.get::<_,String>(0)).optional()?.map(|s|serde_json::from_str::<History>(&s)).transpose()?;
   if let Some(old)=old {
    if old.track_id!=record.track_id||old.started_at_ms!=record.started_at_ms{return Err(invalid("同一历史来源不能更换曲目或开始时间"));}
    record.listened_seconds=record.listened_seconds.max(old.listened_seconds);record.counted|=old.counted;
    if old.finished_at_ms>record.finished_at_ms{record.finished_at_ms=old.finished_at_ms;record.finish_reason=old.finish_reason;}
   }
   crate::accounts::listening::metadata(c,user,&record.track_id,&record.album,&record.album_artist,&record.source,record.started_at_ms)?;
   record.session_id=format!("imported-{}",crate::auth::hash(&format!("{user}\x1f{}",origin.key())));record.imported=true;
   c.execute("INSERT INTO account_history(user_id,event_id,origin_core,origin_user,origin_id,started_at,track_id,data) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(user_id,origin_core,origin_user,origin_id) DO UPDATE SET data=excluded.data",params![user,record.session_id,origin.core,origin.user,origin.id,record.started_at_ms,record.track_id,serde_json::to_string(&record)?])?;
  },
  Record::Stat{data}=>{
   let StatEntry{origin,mut record}=data;
   let old=c.query_row("SELECT data FROM account_stat_origins WHERE user_id=?1 AND origin_core=?2 AND origin_user=?3 AND origin_id=?4",params![user,origin.core,origin.user,origin.id],|r|r.get::<_,String>(0)).optional()?.map(|s|serde_json::from_str::<Stat>(&s)).transpose()?;
   if let Some(old)=&old {
    if old.track_id!=record.track_id||old.minute!=record.minute{return Err(invalid("同一统计来源不能更换曲目或分钟"));}
    if old.revision>0&&record.revision==0{return Ok(());}
    if record.revision>0&&old.revision>0{
     if record.revision<old.revision{return Ok(());}
     if record.revision==old.revision{if &record!=old{return Err(invalid("同一统计修订不能包含不同数据"));}return Ok(());}
     if record.play_seconds<old.play_seconds{return Err(invalid("统计修订不能减少已确认的聆听时间"));}
    }else{record.play_count=record.play_count.max(old.play_count);record.skip_count=record.skip_count.max(old.skip_count);record.play_seconds=record.play_seconds.max(old.play_seconds);}
    record.last_played_at=record.last_played_at.max(old.last_played_at);
   }
   let plays=record.play_count as i64-old.as_ref().map_or(0,|v|v.play_count as i64);let skips=record.skip_count as i64-old.as_ref().map_or(0,|v|v.skip_count as i64);let seconds=record.play_seconds-old.as_ref().map_or(0.,|v|v.play_seconds);
   c.execute("INSERT INTO account_stat_origins(user_id,origin_core,origin_user,origin_id,minute,track_id,data) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(user_id,origin_core,origin_user,origin_id) DO UPDATE SET data=excluded.data",params![user,origin.core,origin.user,origin.id,record.minute,record.track_id,serde_json::to_string(&record)?])?;
   c.execute("INSERT INTO account_track_stats(user_id,track_id,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,track_id) DO UPDATE SET play_count=MAX(0,play_count+excluded.play_count),skip_count=MAX(0,skip_count+excluded.skip_count),play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![user,record.track_id,plays as i64,skips as i64,seconds,record.last_played_at])?;
   c.execute("INSERT INTO account_listening_buckets(user_id,minute,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(user_id,minute,track_id) DO UPDATE SET play_count=MAX(0,play_count+excluded.play_count),skip_count=MAX(0,skip_count+excluded.skip_count),play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![user,record.minute,record.track_id,record.title,record.artist,record.cover_url,plays as i64,skips as i64,seconds,record.last_played_at])?;
  },
  Record::Playlist{data}=>{
   let own=origin_core==core&&origin_user==user&&c.query_row("SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1 AND user_id=?2)",params![data.id,user],|r|r.get::<_,bool>(0))?;
   let id=if data.favorites{format!("favorites-{}",crate::auth::hash(user))}else if own{data.id}else{format!("imported-{}",crate::auth::hash(&format!("{user}\x1f{origin_core}\x1f{origin_user}\x1f{}",data.id)))};
   let old=c.query_row("SELECT tracks,user_id FROM playlists WHERE id=?1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
   let mut tracks=match old{Some((raw,owner))if owner==user=>serde_json::from_str::<Vec<String>>(&raw)?,Some(_)=>return Err(invalid("歌单身份冲突")),None=>Vec::new()};
   let mut seen=tracks.iter().cloned().collect::<std::collections::HashSet<_>>();for track in data.track_ids{if seen.insert(track.clone()){tracks.push(track);}}
   if tracks.len()>10000{return Err(invalid("合并后的歌单不能超过一万首"));}
   c.execute("INSERT INTO playlists(id,name,user_id,tracks,smart_query) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET tracks=excluded.tracks",params![id,data.name,user,serde_json::to_string(&tracks)?,data.smart_query.map(|v|serde_json::to_string(&v)).transpose()?])?;
  },
  Record::Preferences{data}=>{
   let mut value=safe_preferences(data);let old=c.query_row("SELECT value FROM preferences WHERE user_id=?1",[user],|r|r.get::<_,String>(0)).optional()?.map(|v|serde_json::from_str::<Value>(&v)).transpose()?;
   if let(Some(new),Some(Value::Object(old)))=(value.as_object_mut(),old){for(k,v)in old{new.insert(k,v);}}
   c.execute("INSERT INTO preferences(user_id,value) VALUES(?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![user,serde_json::to_string(&value)?])?;
  },
  Record::MusicPreference{data}=>data.apply(c,user)?,
  Record::Manifest{..}=>return Err(invalid("来源头不能作为账号记录导入")),
 }Ok(())
}
