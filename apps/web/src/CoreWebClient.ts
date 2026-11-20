import {RMusicClient} from "@r-music/sdk";
import {coreFetch} from "./secure/coreFetch";

export class CoreWebClient extends RMusicClient {
  override fetchResource(url:string|URL, options:RequestInit={}):Promise<Response>{
    return coreFetch(this.baseUrl,new URL(url,document.baseURI),options);
  }
}

