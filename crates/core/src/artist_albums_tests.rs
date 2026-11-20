use crate::models::Track;

use super::*;
    #[tokio::test]
    async fn discography_uses_verified_artist_and_opens_uncached_release_without_audio() {

        let f = Fixture::new().await;
        f.collection_tracks().await;
        let seed=f.s.db.run(|c|{
            let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],Track::from_row)?;
            t.id="example:Seed01".into();t.recording_id=t.id.clone();t.source="example".into();t.artist="Singer".into();t.provider_data=json!({"artist_credits":[{"id":"Artist01","name":"Singer"}],"artist_mids":["Artist01"],"album_mid":"OldAlbum"});
            crate::library::upsert(c,&t,None,None,0,0,None,None)?;Ok(t)
        }).await.unwrap();
        let artist =
            crate::artist_credits::credits(&seed.source, &seed.artist, &seed.provider_data)[0]
                .id
                .clone();
        let dir = f.s.config.plugin_dir.join("example");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"),json!({"id":"example","capabilities":["metadata","artist_albums_v1","album_catalog_v1","related_catalog_v1"]}).to_string()).unwrap();
        std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);v=json.loads(pathlib.Path(__file__).with_name('responses.json').read_text())[r['method']];print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
        let mut song = seed.clone();
        song.id = "example:NewSong01".into();
        song.recording_id = song.id.clone();
        song.album = "New Album".into();
        song.provider_data["album_mid"] = json!("NewAlbum01");
        let mut responses = json!({"artist_albums":{"source":"example","artist_id":"Artist01","anchor":seed,"page":1,"total":31,"next_page":2,"items":[{"source":"example","release_id":"NewAlbum01","title":"New Album","cover_url":"https://images.example/album.jpg"}]},"album_catalog":{"source":"example","release_id":"NewAlbum01","page":1,"items":[song]},"related_catalog":{"source":"example","matched":true,"kind":"album","anchor":song,"page":1,"collection":{"id":"NewAlbum01"},"artists":[{"id":"Artist01"}],"items":[song]}});
        let file = dir.join("responses.json");
        std::fs::write(&file, responses.to_string()).unwrap();
        let uri = format!("/api/v1/library/catalog/artist/{artist}/albums");
        let (status, data) = f.call("GET", &uri, None).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{data}");
        assert_eq!(
            data["platforms"][0]["items"][0]["id"],
            "release:example:NewAlbum01"
        );
        assert!(data["platforms"][0]["items"][0]["cover_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/artwork/"));
        assert_eq!(data["platforms"][0]["next_page"], 2);
        assert!(crate::library::get_track(&f.s, song.id.clone())
            .await
            .is_err());
        let (status, album) = f
            .call(
                "GET",
                "/api/v1/library/catalog/album/release:example:NewAlbum01",
                None,
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK, "{album}");
        assert_eq!(album["anchor"]["id"], song.id);
        let n =
            f.s.db
                .read(|c| {
                    Ok(c.query_row("SELECT COUNT(*) FROM cache_entries", [], |r| {
                        r.get::<_, i64>(0)
                    })?)
                })
                .await
                .unwrap();
        assert_eq!(n, 0);
        responses["artist_albums"]["artist_id"] = json!("ForeignArtist");
        std::fs::write(&file, responses.to_string()).unwrap();
        let (_, data) = f.call("GET", &(uri + "?refresh=true"), None).await;
        assert!(data["platforms"][0]["items"].as_array().unwrap().is_empty());
        assert!(data["platforms"][0]["notice"]
            .as_str()
            .unwrap()
            .contains("身份"));
        f.s.plugin.stop("example").await;
    }
