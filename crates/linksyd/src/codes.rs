//! Verification codes in a phone's messages, so they can be copied in one click.

/// Words that say a number is a code, in the languages Linksy speaks (lowercase).
const KEYWORDS: &[&str] = &[
    "code", "otp", "verif", "vérif", "passcode", "password", "pin", "one-time", "one time", "2fa", "login", "log in", "sign in", "sign-in",
    "security", "kode", "kaody", "código", "codigo", "codice", "senha", "passwort", "mot de passe", "contraseña", "код", "验证码", "認証",
];

/// The one-time code in `text`: 4 to 8 digits (as 482913, 482-913 or G-482913) near one of
/// [`KEYWORDS`]. Amounts, times, phone numbers and longer numbers aren't codes.
pub fn code_in(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let near: Vec<usize> = KEYWORDS.iter().flat_map(|k| lower.match_indices(k).map(|(i, _)| i)).collect();
    if near.is_empty() {
        return None;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut best: Option<(usize, String)> = None;
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].1.is_ascii_digit() {
            i += 1;
            continue;
        }
        // Digits in groups joined by one space or dash: 482 913, 482-913
        let start = i;
        let mut digits = String::new();
        let mut j = i;
        while j < chars.len() {
            let c = chars[j].1;
            if c.is_ascii_digit() {
                digits.push(c);
                j += 1;
            } else if matches!(c, ' ' | '-') && chars.get(j + 1).is_some_and(|n| n.1.is_ascii_digit()) && chars.get(j.wrapping_sub(1)).is_some_and(|p| p.1.is_ascii_digit()) {
                j += 1;
            } else {
                break;
            }
        }
        i = j;
        let before = start.checked_sub(1).map(|b| chars[b].1);
        let after = chars.get(j).map(|a| a.1);
        let joined_to_word = before.is_some_and(|b| b.is_alphanumeric() && !matches!(b, 'G' | 'g')) || after.is_some_and(|a| a.is_alphanumeric());
        // 12:30, 1,234 or 12.50 continue a number; "code:4829" doesn't
        let digit_before = |k: usize| k.checked_sub(2).is_some_and(|p| chars[p].1.is_ascii_digit());
        let money_or_time = before.is_some_and(|b| matches!(b, '$' | '€' | '£' | '+' | '#') || (matches!(b, '.' | ',' | ':') && digit_before(start)))
            || after.is_some_and(|a| matches!(a, '%' | '€' | ':' | ',') || (a == '.' && chars.get(j + 1).is_some_and(|n| n.1.is_ascii_digit())))
            || chars.get(j).is_some_and(|&(at, _)| followed_by_currency(&text[at..]));
        if !(4..=8).contains(&digits.len()) || joined_to_word || money_or_time {
            continue;
        }
        let at = chars[start].0;
        let distance = near.iter().map(|&k| k.abs_diff(at)).min().unwrap_or(usize::MAX);
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((distance, digits));
        }
    }
    best.map(|(_, code)| code)
}

/// Whether `rest`, what follows a number, starts with a currency ("25000 Ar").
fn followed_by_currency(rest: &str) -> bool {
    let rest = rest.trim_start().to_lowercase();
    ["ar ", "ar.", "ariary", "mga", "eur", "usd", "fcfa", "xof", "dollars", "euros"].iter().any(|c| rest.starts_with(c)) || rest == "ar"
}

#[cfg(test)]
mod tests {
    use super::code_in;

    #[test]
    fn finds_codes() {
        let cases = [
            ("Your verification code is 482913. It expires in 5 minutes.", "482913"),
            ("G-482913 is your Google verification code.", "482913"),
            ("Your WhatsApp code: 123-456. Don't share this code.", "123456"),
            ("Votre code de vérification est 4829", "4829"),
            ("Use 829104 to verify your Microsoft account", "829104"),
            ("Your Uber code is 1234. Never share this code.", "1234"),
            ("Telegram code: 55102. Do not give this code to anyone", "55102"),
            ("Tu código de verificación es 773 401", "773401"),
            ("Ny kaody fanamarinana anao dia 9031", "9031"),
            ("Ваш код: 640218", "640218"),
            ("Order #48213921 shipped. Your delivery PIN is 7710.", "7710"),
            ("Steam login code:55102", "55102"),
        ];
        for (text, code) in cases {
            assert_eq!(code_in(text).as_deref(), Some(code), "{text}");
        }
    }

    #[test]
    fn leaves_other_numbers() {
        let cases = [
            "Are we still on for lunch at 12:30?",
            "Call me at +261 34 12 345 67",
            "Your order #48213921 has shipped",
            "Payment of 25000 Ar received",
            "You spent $1234.50 today",
            "Room 4021 is free",
            "Your password was changed",
            "The code review is at 15:00",
        ];
        for text in cases {
            assert_eq!(code_in(text), None, "{text}");
        }
    }
}
