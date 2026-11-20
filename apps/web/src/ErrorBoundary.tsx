import {Component,type ReactNode} from "react";
export class ErrorBoundary extends Component<{children:ReactNode},{failed:boolean}>{
 state={failed:false};
 static getDerivedStateFromError(){return {failed:true};}
 render(){return this.state.failed?<div className="loading-screen"><h1>页面暂时无法显示</h1><p>重新加载界面后继续，核心里的曲库和播放队列仍会保留。</p><button onClick={()=>this.setState({failed:false})}>重试显示</button><button className="primary" onClick={()=>window.location.reload()}>重新加载</button></div>:this.props.children;}
}
