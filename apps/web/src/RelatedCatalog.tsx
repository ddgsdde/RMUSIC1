import {CatalogLink} from "./CatalogLinks";
import type {RMusicClient,Track} from "@r-music/sdk";
type Props={api:RMusicClient;track:Track;initialKind?:"album"|"artist";play:(track:Track)=>Promise<void>;enqueue:(track:Track)=>Promise<void>};
/** Compatibility entry from song details. Navigation always opens one unified entity page. */
export function RelatedCatalogPanel({track}:Props){
 return <section className="entity-caption-links"><h3>专辑</h3><CatalogLink track={track} album/><h3>参与艺术家</h3><CatalogLink track={track}/></section>;
}
export function RelatedCatalogButton({track}:Props){
 return <span className="entity-caption-links"><CatalogLink track={track} album/> · <CatalogLink track={track}/></span>;
}
