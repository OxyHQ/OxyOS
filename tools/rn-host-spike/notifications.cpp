#include "notifications.h"

#include <cstring>

namespace oxyos {

bool NotificationTexts::observe(const OxyOSEvent& event) {
  if (event.kind != OXYOS_EVENT_NOTIFICATION_TEXT) return false;
  auto& text = texts_[event.code][{event.height, event.width}];
  if (event.detail == 0) text.clear();
  text.append(event.text, strnlen(event.text, sizeof(event.text)));
  return true;
}

NotificationTexts::Texts NotificationTexts::take(uint32_t notification) {
  auto found = texts_.find(notification);
  if (found == texts_.end()) return {};
  auto texts = std::move(found->second);
  texts_.erase(found);
  return texts;
}

std::string notificationText(const NotificationTexts::Texts& texts, uint32_t field, uint32_t action) {
  auto found = texts.find({field, action});
  return found == texts.end() ? std::string() : found->second;
}

folly::dynamic notificationResponse(const OxyOSEvent& response, const NotificationTexts::Texts& texts) {
  auto text = [&](uint32_t field, uint32_t action = 0) { return notificationText(texts, field, action); };
  return folly::dynamic::object("notification", response.code)("action", response.width)(
      "identifier", text(OXYOS_NOTIFICATION_FIELD_IDENTIFIER))("title", text(OXYOS_NOTIFICATION_FIELD_TITLE))(
      "subtitle", text(OXYOS_NOTIFICATION_FIELD_SUBTITLE))("body", text(OXYOS_NOTIFICATION_FIELD_BODY))(
      "data", text(OXYOS_NOTIFICATION_FIELD_DATA))("actionId", text(OXYOS_NOTIFICATION_FIELD_ACTION_ID, response.width))(
      "reply", text(OXYOS_NOTIFICATION_FIELD_REPLY));
}

folly::dynamic notificationResult(const OxyOSEvent& result) {
  const char* outcome = result.detail == OXYOS_NOTIFICATION_POSTED     ? "posted"
                        : result.detail == OXYOS_NOTIFICATION_THROTTLED ? "throttled"
                                                                         : "refused";
  return folly::dynamic::object("serial", result.width)("notification", result.code)("outcome", outcome);
}

}  // namespace oxyos
