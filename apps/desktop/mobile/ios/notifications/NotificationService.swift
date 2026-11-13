import CryptoKit
import Foundation
import Security
import UserNotifications

struct Shown {
  let target: String
  let sender: String
  let text: String
  let conversation: String?
}

enum Previews {
  static let service = "chat.commy.mobile.previews"
  static let account = "wake-previews"
  static let keepLines = 4
  private static let aad = Data("commy-wake-preview/v1".utf8)
  private static let nonceBytes = 12
  private static let tagBytes = 16
  private static let id = try! NSRegularExpression(pattern: "^[0-9a-fA-F-]{1,64}$")

  static func held() -> [String: Any]? {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    var found: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess, let data = found as? Data else { return nil }
    return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
  }

  static func open(_ sealed: String, held: [String: Any]) -> Shown? {
    guard let bytes = Data(base64Encoded: sealed), bytes.count > nonceBytes + tagBytes,
      let keys = held["keys"] as? [[String: Any]],
      let nonce = try? AES.GCM.Nonce(data: Data(bytes.prefix(nonceBytes))),
      let box = try? AES.GCM.SealedBox(
        nonce: nonce, ciphertext: Data(bytes.dropFirst(nonceBytes).dropLast(tagBytes)), tag: Data(bytes.suffix(tagBytes)))
    else { return nil }
    for entry in keys {
      guard let encoded = entry["k"] as? String, let raw = Data(base64Encoded: encoded), raw.count == 32,
        let plain = try? AES.GCM.open(box, using: SymmetricKey(data: raw), authenticating: aad)
      else { continue }
      guard let text = String(data: plain, encoding: .utf8),
        let preview = (try? JSONSerialization.jsonObject(with: Data(text.trimmingCharacters(in: .whitespaces).utf8))) as? [String: Any]
      else { return nil }
      return shown(preview, server: nonEmpty(entry["s"] as? String), held: held)
    }
    return nil
  }

  private static func valid(_ text: String) -> Bool {
    id.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
  }

  private static func nonEmpty(_ text: String?) -> String? {
    guard let text = text, !text.isEmpty else { return nil }
    return text
  }

  private static func shown(_ preview: [String: Any], server: String?, held: [String: Any]) -> Shown? {
    let author = preview["a"] as? String ?? ""
    let call = preview["k"] as? Bool ?? false
    let ended = preview["e"] as? Bool ?? false
    let text = call && !ended ? "Calling you" : preview["t"] as? String ?? ""
    guard valid(author), author != held["me"] as? String else { return nil }
    guard let server = server else {
      let name = nonEmpty((held["users"] as? [String: Any])?[author] as? String) ?? "Someone"
      return Shown(target: "dm:\(author)", sender: name, text: text, conversation: nil)
    }
    let channel = preview["c"] as? String ?? ""
    guard valid(server), valid(channel) else { return nil }
    let label = (held["servers"] as? [String: Any])?[server] as? [String: Any]
    let name = nonEmpty((label?["m"] as? [String: Any])?[author] as? String) ?? "Someone"
    let serverName = label?["n"] as? String ?? ""
    let channelName = (label?["c"] as? [String: Any])?[channel] as? String ?? ""
    let conversation: String?
    if label?["g"] as? Bool == true {
      conversation = nonEmpty(serverName)
    } else if !channelName.isEmpty && !serverName.isEmpty {
      conversation = "#\(channelName), \(serverName)"
    } else {
      conversation = nonEmpty(serverName)
    }
    return Shown(target: "channel:\(server):\(channel)", sender: name, text: text, conversation: conversation)
  }

  static func body(_ lines: [Shown]) -> String {
    let senders = Set(lines.map { $0.sender })
    let texts = lines.map { line -> String in
      let text = line.text.isEmpty ? "New message" : line.text
      return senders.count > 1 ? "\(line.sender): \(text)" : text
    }
    return texts.joined(separator: "\n")
  }
}

enum Sounds {
  static let message = UNNotificationSound(named: UNNotificationSoundName("commy_message.wav"))
  static let direct = UNNotificationSound(named: UNNotificationSoundName("commy_dm.wav"))

  static func sound(for target: String?) -> UNNotificationSound {
    target?.hasPrefix("dm:") == true ? direct : message
  }
}

final class NotificationService: UNNotificationServiceExtension {
  private var deliver: ((UNNotificationContent) -> Void)?
  private var content: UNMutableNotificationContent?

  override func didReceive(
    _ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
  ) {
    let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
    deliver = contentHandler
    self.content = content
    fill(content, from: request.content.userInfo)
    finish()
  }

  override func serviceExtensionTimeWillExpire() {
    finish()
  }

  private func finish() {
    guard let deliver = deliver, let content = content else { return }
    self.deliver = nil
    deliver(content)
  }

  private func useCommySound(_ content: UNMutableNotificationContent, for target: String?) {
    guard content.sound != nil else { return }
    content.sound = Sounds.sound(for: target)
  }

  private func fill(_ content: UNMutableNotificationContent, from info: [AnyHashable: Any]) {
    useCommySound(content, for: nil)
    guard let held = Previews.held() else { return }
    var sealed: [String] = []
    if let list = info["previews"] as? String {
      sealed = list.split(separator: ",").map(String.init)
    } else if let one = info["preview"] as? String {
      sealed = [one]
    }
    let shown = sealed.compactMap { Previews.open($0, held: held) }
    guard let newest = shown.last else { return }
    let lines = Array(shown.filter { $0.target == newest.target }.suffix(Previews.keepLines))
    content.title = newest.sender
    content.subtitle = newest.conversation ?? ""
    content.body = Previews.body(lines)
    content.threadIdentifier = newest.target
    useCommySound(content, for: newest.target)
    var userInfo = content.userInfo
    userInfo["target"] = newest.target
    content.userInfo = userInfo
  }
}
