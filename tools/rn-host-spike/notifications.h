// Notifications (docs/adr/0026-notifications.mdx), as the Bloom host gathers them from the
// compositor's events. A notification's texts come in chunks (NOTIFICATION_TEXT), by field and
// action, before the event that says it is whole: the shell's NOTIFICATION, an application's
// NOTIFICATION_RESPONSE. The application posts through expo-notifications' API, which the host's
// adapter (sample/oxyos/expo-adapter.js) implements over the OxyOSNotifications module.
#pragma once

#include <oxyos_client.h>

#include <folly/dynamic.h>

#include <cstdint>
#include <map>
#include <string>
#include <utility>

namespace oxyos {

class NotificationTexts {
 public:
  using Texts = std::map<std::pair<uint32_t, uint32_t>, std::string>;
  // Takes in a NOTIFICATION_TEXT's chunk; false for any other event.
  bool observe(const OxyOSEvent& event);
  // The notification's texts received so far, by (OXYOS_NOTIFICATION_FIELD_*, action), forgotten.
  Texts take(uint32_t notification);
  void forget(uint32_t notification) { texts_.erase(notification); }

 private:
  std::map<uint32_t, Texts> texts_;
};

// One text of `texts`, empty if none came.
std::string notificationText(const NotificationTexts::Texts& texts, uint32_t field, uint32_t action = 0);

// A response for JavaScript, from its texts and NOTIFICATION_RESPONSE: { notification, action,
// identifier, title, subtitle, body, data, actionId, reply }.
folly::dynamic notificationResponse(const OxyOSEvent& response, const NotificationTexts::Texts& texts);

// What came of a post, as JavaScript is told it: { serial, notification, outcome: 'posted' |
// 'refused' | 'throttled' }.
folly::dynamic notificationResult(const OxyOSEvent& result);

}  // namespace oxyos
