use super::*;
use std::sync::Arc;
/// Android document providers supply seekable descriptors, never fabricated filesystem paths.
pub trait DocumentStorage:Send+Sync {
    fn create(&self,tree:&str,name:&str)->Result<String>;
    fn open(&self,uri:&str,write:bool)->Result<fs::File>;
    fn remove(&self,uri:&str)->Result<()>;
}
#[derive(Clone)]
pub struct CacheFile {path:PathBuf,_descriptor:Option<Arc<fs::File>>}
impl CacheFile {
    pub(super) fn rewind(&self)->Result<()> {if let Some(file)=&self._descriptor{file.try_clone()?.seek(SeekFrom::Start(0))?;}Ok(())}
    fn local(path:PathBuf)->Self{Self{path,_descriptor:None}}
    #[cfg(unix)]
    fn document(mut file:fs::File)->Result<Self>{
        use std::os::fd::AsRawFd;
        file.seek(SeekFrom::Start(0)).context("此存储位置不支持离线音乐随机读取，请选择本机或SD卡目录")?;
        let directory=if cfg!(target_os="android")||cfg!(target_os="linux"){"/proc/self/fd"}else{"/dev/fd"};
        Ok(Self{path:PathBuf::from(format!("{directory}/{}",file.as_raw_fd())),_descriptor:Some(Arc::new(file))})
    }
    #[cfg(not(unix))]
    fn document(_:fs::File)->Result<Self>{bail!("此平台不使用Android文档目录")}
}
impl std::ops::Deref for CacheFile {type Target=Path;fn deref(&self)->&Path{&self.path}}
impl AsRef<Path> for CacheFile {fn as_ref(&self)->&Path{&self.path}}
impl CacheStore {
    pub fn document_storage(&mut self,storage:Arc<dyn DocumentStorage>){self.documents=Some(storage);}
    pub fn active_count(&self,id:&str)->u32{self.active.get(id).copied().unwrap_or(0)}
    pub fn private_file(&self,entry:&CacheEntry)->PathBuf{self.root.join(format!("{}.{}",entry.id,entry.format))}
    pub(super) fn read_file(&self,entry:&CacheEntry)->Result<CacheFile>{
        match &entry.document_uri {Some(uri)=>CacheFile::document(self.documents.as_ref().context("离线存储尚未连接")?.open(uri,false)?),None=>Ok(CacheFile::local(self.path(entry)))}
    }
    pub fn ensure_download(&mut self,mut entry:CacheEntry,tree:Option<&str>)->Result<()> {
        if self.manifest.entries.contains_key(&entry.id)||tree.is_none(){return self.ensure_entry(entry);}
        let storage=self.documents.clone().context("离线目录尚未连接")?;let tree=tree.unwrap();
        let title=entry.title.as_deref().unwrap_or(&entry.track_id).chars().take(100).collect::<String>();
        let name=format!("{} [{}].{}",title,&entry.id[..entry.id.len().min(12)],entry.format);
        let uri=storage.create(tree,&name)?;entry.document_uri=Some(uri.clone());entry.document_tree=Some(tree.into());
        if let Err(error)=self.ensure_entry(entry){let _=storage.remove(&uri);return Err(error);}Ok(())
    }
    pub fn relocate(&mut self,id:&str,expected_uri:Option<&str>,uri:Option<String>,tree:Option<String>)->Result<bool>{
        if self.active_count(id)!=1{return Ok(false);}
        let old=self.manifest.entries.get(id).context("离线文件已移除")?.clone();
        if !old.complete||old.document_uri.as_deref()!=expected_uri{return Ok(false);}
        let entry=self.manifest.entries.get_mut(id).unwrap();entry.document_uri=uri;entry.document_tree=tree;
        if let Err(error)=self.persist(){self.manifest.entries.insert(id.into(),old);return Err(error);}Ok(true)
    }
    pub fn discard_copy(&self,entry:&CacheEntry)->Result<()> {
        if let Some(uri)=&entry.document_uri{self.documents.as_ref().context("离线目录尚未连接")?.remove(uri)}
        else{let path=self.path(entry);if path.exists(){fs::remove_file(path)?;}Ok(())}
    }
}

#[cfg(all(test,unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool,Ordering};
    struct Provider{root:PathBuf,granted:AtomicBool}
    impl DocumentStorage for Provider {
        fn create(&self,_:&str,name:&str)->Result<String>{let path=self.root.join(name);fs::File::create(path)?;Ok(format!("content://fixture/{name}"))}
        fn open(&self,uri:&str,write:bool)->Result<fs::File>{anyhow::ensure!(self.granted.load(Ordering::Relaxed),"directory permission revoked");Ok(OpenOptions::new().read(true).write(write).open(self.root.join(uri.strip_prefix("content://fixture/").unwrap()))?)}
        fn remove(&self,uri:&str)->Result<()>{fs::remove_file(self.root.join(uri.strip_prefix("content://fixture/").unwrap()))?;Ok(())}
    }
    fn entry()->CacheEntry {CacheEntry{id:"external".into(),profile_id:"core".into(),owner:None,track_id:"song".into(),title:Some("Song".into()),artist:None,recording_id:"recording".into(),format:"flac".into(),total_bytes:8,ranges:vec![],complete:false,pinned:false,manual_download:true,last_access:0,quality_rank:1,sha256:None,sample_rate:48000,channels:6,bit_depth:24,bitrate:0,verified:false,source_etag:Some("\"stable\"".into()),document_uri:None,document_tree:None}}
    #[test]
    fn document_download_resumes_and_survives_restart_and_lost_permission(){
        let root=tempfile::tempdir().unwrap();let directory=tempfile::tempdir().unwrap();let provider=Arc::new(Provider{root:directory.path().into(),granted:AtomicBool::new(true)});
        let mut cache=CacheStore::open(root.path()).unwrap();cache.document_storage(provider.clone());cache.ensure_download(entry(),Some("content://folder")).unwrap();
        cache.write_range("external",0,b"abcd").unwrap();assert_eq!(cache.entry_bytes(&cache.manifest.entries["external"]),4);drop(cache);
        let mut cache=CacheStore::open(root.path()).unwrap();cache.document_storage(provider.clone());assert_eq!(cache.manifest.entries["external"].ranges[0].end,4);
        cache.write_range("external",4,b"efgh").unwrap();let file=cache.complete_file("external").unwrap();assert_eq!(fs::read(&file).unwrap(),b"abcdefgh");drop(file);assert_eq!(cache.manual_bytes(),8);
        assert!(!root.path().join("external.flac").exists());drop(cache);
        provider.granted.store(false,Ordering::Relaxed);let mut cache=CacheStore::open(root.path()).unwrap();cache.document_storage(provider.clone());assert!(cache.complete_file("external").is_err());assert!(cache.manifest.entries["external"].complete);
        provider.granted.store(true,Ordering::Relaxed);let file=cache.complete_file("external").unwrap();assert_eq!(fs::read(&file).unwrap(),b"abcdefgh");
    }
    #[test]
    fn complete_document_ranges_can_finish_verification_after_restart(){
        let root=tempfile::tempdir().unwrap();let directory=tempfile::tempdir().unwrap();let provider=Arc::new(Provider{root:directory.path().into(),granted:AtomicBool::new(true)});
        let mut cache=CacheStore::open(root.path()).unwrap();cache.document_storage(provider.clone());cache.ensure_download(entry(),Some("content://folder")).unwrap();
        assert!(cache.finish_download("external").is_err());
        let row=cache.manifest.entries.get_mut("external").unwrap();let mut file=provider.open(row.document_uri.as_ref().unwrap(),true).unwrap();file.write_all(b"abcdefgh").unwrap();file.sync_all().unwrap();row.ranges=vec![ByteRange{start:0,end:8}];cache.persist().unwrap();drop(cache);
        let mut cache=CacheStore::open(root.path()).unwrap();cache.document_storage(provider);cache.finish_download("external").unwrap();assert!(cache.manifest.entries["external"].complete);assert!(cache.complete_file("external").is_ok());
    }
    #[test]
    fn moving_requires_an_idle_file_and_preserves_its_metadata(){
        let root=tempfile::tempdir().unwrap();let mut cache=CacheStore::open(root.path()).unwrap();cache.ensure_entry(entry()).unwrap();cache.write_range("external",0,b"abcdefgh").unwrap();
        let original=cache.manifest.entries["external"].clone();cache.protect("external");cache.protect("external");
        assert!(!cache.relocate("external",None,Some("content://new/file".into()),Some("content://new".into())).unwrap());
        cache.release("external");assert!(cache.relocate("external",None,Some("content://new/file".into()),Some("content://new".into())).unwrap());
        let moved=&cache.manifest.entries["external"];assert_eq!(moved.sha256,original.sha256);assert_eq!(moved.recording_id,original.recording_id);assert_eq!(moved.ranges,original.ranges);assert_eq!(moved.channels,6);
    }
}
