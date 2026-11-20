import type {SignalGrade} from "./signalGrade";
export function SignalLight({level}:{level:SignalGrade["level"]}){
 return <span className={`signal-light ${level}`} aria-hidden="true"/>;
}
