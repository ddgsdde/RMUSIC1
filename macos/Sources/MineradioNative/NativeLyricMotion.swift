import Foundation

/// Persistent two-stage track/row easing from 12-lyrics-row-layers.js (settled ease 0.135).
/// Glyph readiness never owns the scroll clock; only new tracks reset it.
struct NativeLyricMotion {
    struct Pose {
        var y: Float
        var z: Float
        var scale: Float
        var opacity: Float = 0
        var active: Float = 0
    }
    private(set) var offset: Float?
    private var poses: [Int:Pose] = [:]
    private var progress: [Int:Float] = [:]
    var count: Int { poses.count }
    mutating func reset() { offset = nil; poses.removeAll(); progress.removeAll() }
    mutating func retain(_ keys: Set<Int>) { poses = poses.filter { keys.contains($0.key) }; progress = progress.filter { keys.contains($0.key) } }
    mutating func wordProgress(key: Int, target: Float, karaoke: Bool, style: NativeLyricStyle, dt: Float) -> Float {
        let old = progress[key] ?? target
        let ease = abs(target-old) > 0.42 ? max(0.52,style.progressEase*1.55) : style.progressEase
        let next = karaoke ? target : old+(target-old)*(1-pow(1-min(0.99,ease),max(0,dt*60)))
        progress[key] = next;return next
    }
    mutating func step(target: Float, slot: Float, dt: Float) {
        guard let previous = offset else { offset = target; return }
        let frames = max(0,min(3,dt*60)), limit = max(0.25,slot)*0.68*frames
        let step = (target-previous)*(1-pow(1-0.135*1.16,frames))
        offset = previous+min(limit,max(-limit,step))
    }
    mutating func pose(key: Int, y: Float, z: Float, scale: Float, opacity: Float, active: Bool, slot: Float, lineStep: Float, dt: Float) -> Pose {
        var pose = poses[key] ?? Pose(y:y,z:z,scale:scale)
        let frames = max(0,min(3,dt*60)), ease = 1-pow(1-0.135,frames)
        let limit = max(0.25,slot)*lineStep*0.66*frames
        pose.y += min(limit,max(-limit,(y-pose.y)*ease))
        pose.z += (z-pose.z)*ease
        pose.scale += (scale-pose.scale)*ease
        pose.opacity += (opacity-pose.opacity)*ease
        pose.active += ((active ? 1 : 0)-pose.active)*(1-pow(1-(active ? 0.34 : 0.62),frames))
        poses[key] = pose
        return pose
    }
}
