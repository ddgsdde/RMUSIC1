import Foundation

struct PlaybackSettingsDraft {
  static let balancedFadeIn = 80
  static let balancedFadeOut = 120
  static var balancedTransitions: JSONValue {
    .object(["fade_in_ms": .number(Double(balancedFadeIn)), "fade_out_ms": .number(Double(balancedFadeOut))])
  }
  enum Filter: String, CaseIterable {
    case peak, lowShelf = "low_shelf", highShelf = "high_shelf", lowPass = "low_pass"
    case highPass = "high_pass", bandPass = "band_pass", notch, allPass = "all_pass"
    var hasGain: Bool { self == .peak || self == .lowShelf || self == .highShelf }
    var isCut: Bool { self == .lowPass || self == .highPass }
    var label: String {
      switch self {
      case .peak: "峰值"
      case .lowShelf: "低架"
      case .highShelf: "高架"
      case .lowPass: "低通"
      case .highPass: "高通"
      case .bandPass: "带通"
      case .notch: "陷波"
      case .allPass: "全通"
      }
    }
  }
  struct Band: Identifiable {
    let id = UUID()
    var enabled = true
    var kind: Filter = .peak
    var frequency = 1000.0
    var gain = 0.0
    var q = 1.0
    var slope = 12
    var value: JSONValue {
      var band: [String: JSONValue] = [
        "enabled": .bool(enabled), "frequency": .number(frequency),
        "gain_db": .number(gain), "q": .number(q), "kind": .string(kind.rawValue)
      ]
      if kind.isCut && slope != 12 { band["slope_db_oct"] = .number(Double(slope)) }
      return .object(band)
    }
    static func decode(_ value: JSONValue) -> Self {
      Band(enabled: value["enabled"].boolean, kind: Filter(rawValue: value["kind"].text) ?? .peak,
        frequency: value["frequency"].number, gain: value["gain_db"].number, q: value["q"].number,
        slope: value["slope_db_oct"].integer == 0 ? 12 : value["slope_db_oct"].integer)
    }
  }
  struct EQStage {
    var bands: [Band] = []
    var bypass = false
    init(bands: [Band] = [], bypass: Bool = false) { self.bands = bands; self.bypass = bypass }
    init(_ value: JSONValue) { bands = value["bands"].array.map(Band.decode); bypass = value["bypass"].boolean }
    var value: JSONValue { .object(["bands": .array(bands.map(\.value)), "bypass": .bool(bypass)]) }
  }
  var extraEQ: [EQStage] = []
  var installedModules: [String]?
  var allEQ: [EQStage] {
    get { [EQStage(bands:bands,bypass:eqBypass)] + extraEQ }
    set { bands = newValue.first?.bands ?? []; eqBypass = newValue.first?.bypass ?? false; extraEQ = Array(newValue.dropFirst()) }
  }
  var totalBands: Int { allEQ.reduce(0) { $0 + $1.bands.count } }
  func modules(portable:Bool) -> Set<String> {
    var names = Set(installedModules ?? (portable ? ["loudness", "fades"] : ["eq", "loudness", "fades"]))
    if !bands.isEmpty || !extraEQ.isEmpty { names.insert("eq") }
    if preamp != 0 || balance != 0 { names.insert("gain") }
    if sampleRate != 0 { names.insert("rate") }
    if stereo != nil { names.insert("stereo") }
    if !(original["convolution"]?["response_ids"].array.isEmpty ?? true) { names.insert("convolution") }
    if normalize { names.insert("loudness") }
    if fadeIn != 0 || fadeOut != 0 { names.insert("fades") }
    return names
  }
  mutating func editEQ(at index: Int,_ change: (inout EQStage) -> Void) {
    guard allEQ.indices.contains(index) else { return }
    var stages = allEQ
    change(&stages[index]); allEQ = stages
  }
  struct Stereo {
    var crossfeed = 0.0
    var crossover = 700.0
    var mono = false
    var invertLeft = false
    var invertRight = false
    var value: JSONValue { .object([
      "crossfeed": .number(crossfeed), "crossover_hz": .number(crossover), "mono": .bool(mono),
      "invert_left": .bool(invertLeft), "invert_right": .bool(invertRight)
    ]) }
  }
  var stereo: Stereo?
  var eqBypass = false
  var enabled = false, normalize = false
  var preamp = 0.0, balance = 0.0, target = -14.0
  static let sampleRates = [8000,11025,16000,22050,24000,32000,44100,48000,88200,96000,176400,192000,352800,384000,705600,768000]
  var sampleRate = 0
  var fadeIn = 0, fadeOut = 0
  var bands: [Band] = []
  var original: [String: JSONValue] = [:]
  init(_ zone: Zone? = nil) {
    guard let zone else { return }
    original = zone.dsp?.object ?? [:]
    enabled = original["enabled"]?.boolean ?? false
    normalize = original["normalize"]?.boolean ?? false
    preamp = original["preamp_db"]?.number ?? 0
    balance = original["balance"]?.number ?? 0
    target = original["target_lufs"]?.number ?? -14
    bands = (original["bands"]?.array ?? []).map(Band.decode)
    extraEQ = original["extra_eq"]?.array.map(EQStage.init) ?? []
    installedModules = original["modules"]?.array.compactMap(\.string)
    sampleRate = original["sample_rate"]?.integer ?? 0
    eqBypass = original["eq_bypass"]?.boolean ?? false
    if let options = original["stereo"], options.object != nil {
      stereo = Stereo(crossfeed: options["crossfeed"].number,
        crossover: options["crossover_hz"] == .null ? 700 : options["crossover_hz"].number,
        mono: options["mono"].boolean, invertLeft: options["invert_left"].boolean, invertRight: options["invert_right"].boolean)
    }
    fadeIn = zone.transitions?["fade_in_ms"].integer ?? 0
    fadeOut = zone.transitions?["fade_out_ms"].integer ?? 0
  }
  mutating func applyEqualizerPreset(_ preset: JSONValue) {
    enabled = true
    eqBypass = false
    preamp = preset["preamp_db"].number
    bands = preset["bands"].array.map(Band.decode)
  }
  func payload(rate _: Int) throws -> JSONValue {
    guard preamp.isFinite, (-60...24).contains(preamp), balance.isFinite,
      (-1...1).contains(balance), target.isFinite, (-40 ... -5).contains(target), extraEQ.count <= 3, totalBands <= 64, allEQ.allSatisfy({$0.bands.count <= 32}),
      sampleRate == 0 || Self.sampleRates.contains(sampleRate), (0...10000).contains(fadeIn), (0...10000).contains(fadeOut),
      stereo.map({ (0...0.5).contains($0.crossfeed) && (200...2000).contains($0.crossover) }) ?? true,
      allEQ.flatMap(\.bands).allSatisfy({
        $0.frequency.isFinite && (10...192000).contains($0.frequency)
          && $0.gain.isFinite && (-30...30).contains($0.gain) && $0.q.isFinite
          && (0.1...30).contains($0.q) && ($0.slope == 12 || $0.kind.isCut && [24,36,48].contains($0.slope))
      })
    else { throw RuntimeFailure(message: "请检查增益、频率、Q 值和淡入淡出范围。") }
    var dsp = original
    dsp["enabled"] = .bool(enabled)
    dsp["normalize"] = .bool(normalize)
    dsp["preamp_db"] = .number(preamp)
    dsp["balance"] = .number(balance)
    dsp["target_lufs"] = .number(target)
    dsp["sample_rate"] = sampleRate > 0 ? .number(Double(sampleRate)) : nil
    dsp["eq_bypass"] = eqBypass ? .bool(true) : nil
    dsp["stereo"] = stereo?.value
    dsp["bands"] = .array(bands.map(\.value))
    dsp["extra_eq"] = extraEQ.isEmpty ? nil : .array(extraEQ.map(\.value))
    dsp["modules"] = installedModules.map { .array($0.map(JSONValue.string)) }
    return .object([
      "dsp": .object(dsp),
      "transitions": .object([
        "fade_in_ms": .number(Double(fadeIn)), "fade_out_ms": .number(Double(fadeOut)),
      ]),
    ])
  }
}
