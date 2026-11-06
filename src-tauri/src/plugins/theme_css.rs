//! Independently implemented CSS compiler. Only package assets and public parts cross this boundary.
use crate::contracts::{ApiError, Result};
use cssparser::{Parser, ParserInput, Token as CssToken};
use lightningcss::{
    declaration::DeclarationBlock,
    properties::{
        Property,
        custom::{Function, Token, TokenOrValue},
        font::FontFamily,
    },
    rules::{
        CssRule, CssRuleList,
        font_face::{FontFaceProperty, Source},
        keyframes::KeyframesName,
    },
    stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
    traits::{Parse, ToCss},
    values::url::Url,
    visitor::{Visit, VisitTypes, Visitor},
};
use static_self::IntoOwned;
use std::collections::{BTreeMap, BTreeSet};

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("THEME_CSS", message)
}

// CSS tokenization decodes escapes before checking names. Nested :root is rejected;
// a root selector may never traverse a sibling, even after a descendant compound.
fn check_selector(
    input: &mut Parser<'_, '_>,
    nested: bool,
    root: &mut bool,
    sibling: &mut bool,
) -> Result<()> {
    while !input.is_exhausted() {
        let token = input
            .next()
            .map_err(|e| invalid(format!("选择器: {e:?}")))?
            .clone();
        match token {
            CssToken::IDHash(_) | CssToken::Hash(_) | CssToken::Delim('.') => {
                return Err(invalid(
                    "请用公共 data-h-part，不允许应用 ID 或私有类选择器。",
                ));
            }
            CssToken::Ident(name)
                if name.eq_ignore_ascii_case("html") || name.eq_ignore_ascii_case("body") =>
            {
                return Err(invalid("请用 :root，不允许 html/body 全局选择器。"));
            }
            CssToken::Ident(name) if name.eq_ignore_ascii_case("root") => {
                if nested {
                    return Err(invalid("嵌套选择器中不允许 :root。"));
                }
                *root = true;
            }
            CssToken::Delim('+' | '~' | '|') => *sibling = true,
            CssToken::SquareBracketBlock => {
                input
                    .parse_nested_block(|p| {
                        let name = p.expect_ident_cloned()?;
                        let attribute = name.as_ref();
                        if ![
                            "data-h-part",
                            "data-h-layer",
                            "data-h-variant",
                            "data-h-scope",
                            "data-h-page",
                            "data-h-sidebar",
                            "disabled",
                            "hidden",
                            "type",
                            "role",
                            "aria-selected",
                            "aria-expanded",
                            "aria-checked",
                            "aria-current",
                            "aria-disabled",
                        ]
                        .contains(&attribute)
                        {
                            return Err(
                                p.new_custom_error::<String, String>("未公开的属性选择器".into())
                            );
                        }
                        if attribute.starts_with("data-h-") {
                            p.expect_delim('=')?;
                            let value = p.expect_ident_or_string()?.to_string();
                            if !super::themes::public_value(attribute, &value) {
                                return Err(p.new_custom_error::<String, String>(format!(
                                    "未知部位或状态 {attribute}={value}"
                                )));
                            }
                        }
                        while !p.is_exhausted() {
                            p.next()?;
                        }
                        Ok(())
                    })
                    .map_err(|e| ApiError::new("THEME_PART", format!("属性选择器: {e:?}")))?;
            }
            CssToken::Function(_) | CssToken::ParenthesisBlock => {
                input
                    .parse_nested_block(|p| {
                        check_selector(p, true, root, sibling)
                            .map_err(|e| p.new_custom_error::<ApiError, ApiError>(e))
                    })
                    .map_err(|e| invalid(format!("选择器: {e:?}")))?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn scoped(selector: &str, scope: &str) -> Result<String> {
    let mut input = ParserInput::new(selector);
    let mut parser = Parser::new(&mut input);
    let (mut root, mut sibling) = (false, false);
    check_selector(&mut parser, false, &mut root, &mut sibling)?;
    if root {
        if !selector.starts_with(":root") || selector.matches(":root").count() != 1 || sibling {
            return Err(invalid(
                ":root 只允许位于首个复合选择器，且不能包含同级组合符。",
            ));
        }
        Ok(selector.replacen(":root", scope, 1))
    } else {
        Ok(format!("{scope} {selector}"))
    }
}

fn names(
    rules: &CssRuleList<'_>,
    fonts: &mut BTreeMap<String, String>,
    animations: &mut BTreeSet<String>,
    namespace: &str,
) -> Result<()> {
    for rule in &rules.0 {
        match rule {
            CssRule::Media(r) => names(&r.rules, fonts, animations, namespace)?,
            CssRule::Supports(r) => names(&r.rules, fonts, animations, namespace)?,
            CssRule::Keyframes(r) => {
                animations.insert(match &r.name {
                    KeyframesName::Custom(s) => s.to_string(),
                    KeyframesName::Ident(s) => s.0.to_string(),
                });
            }
            CssRule::FontFace(r) => {
                let mut family = false;
                let mut source = false;
                for p in &r.properties {
                    match p {
                        FontFaceProperty::FontFamily(f) => {
                            if matches!(f, FontFamily::Generic(_)) {
                                return Err(invalid("包内字体不能占用系统通用字体名。"));
                            }
                            let name = f
                                .to_css_string(PrinterOptions::default())
                                .map_err(|e| invalid(e.to_string()))?;
                            let index = fonts.len();
                            fonts
                                .entry(name)
                                .or_insert_with(|| format!("\"{namespace}-font-{index}\""));
                            family = true;
                        }
                        FontFaceProperty::Source(s) => {
                            if s.is_empty() || s.iter().any(|s| !matches!(s, Source::Url(u) if u.url.url.ends_with(".woff2") && u.tech.is_empty())) {
                                return Err(invalid("@font-face 只接受包内 WOFF2，不接受 local()/tech()。"));
                            }
                            source = true;
                        }
                        FontFaceProperty::Custom(p) if p.name.as_ref() == "font-display" => {}
                        FontFaceProperty::Custom(_) => {
                            return Err(invalid("不支持此 @font-face 描述符。"));
                        }
                        _ => {}
                    }
                }
                if !family || !source {
                    return Err(invalid("@font-face 缺少 font-family/src。"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct Compiler<'a, F> {
    resolve: &'a mut F,
    fonts: &'a BTreeMap<String, String>,
    animations: &'a BTreeSet<String>,
    namespace: &'a str,
}
impl<'i, F: FnMut(&str) -> Result<String>> Visitor<'i> for Compiler<'_, F> {
    type Error = ApiError;
    fn visit_types(&self) -> VisitTypes {
        lightningcss::visit_types!(RULES | PROPERTIES | URLS | IMAGES | FUNCTIONS | TOKENS)
    }
    fn visit_rule(&mut self, rule: &mut CssRule<'i>) -> Result<()> {
        match rule {
            CssRule::Style(r) if !r.rules.0.is_empty() => {
                return Err(invalid("首版不支持 CSS nesting，请展开选择器。"));
            }
            CssRule::Style(_)
            | CssRule::Media(_)
            | CssRule::Supports(_)
            | CssRule::FontFace(_)
            | CssRule::Keyframes(_) => {}
            _ => {
                return Err(invalid(
                    "只支持 @media、@supports、@font-face、@keyframes；不允许 @import。",
                ));
            }
        }
        if let CssRule::FontFace(r) = rule {
            for p in &mut r.properties {
                if let FontFaceProperty::FontFamily(f) = p {
                    self.family(f)?;
                }
            }
        }
        if let CssRule::Keyframes(r) = rule {
            match &mut r.name {
                KeyframesName::Custom(name) => *name = format!("{}-{name}", self.namespace).into(),
                KeyframesName::Ident(name) => {
                    name.0 = format!("{}-{}", self.namespace, name.0).into()
                }
            }
        }
        rule.visit_children(self)
    }
    fn visit_declaration_block(&mut self, declarations: &mut DeclarationBlock<'i>) -> Result<()> {
        if !declarations.important_declarations.is_empty() {
            return Err(invalid("主题不能使用 !important。"));
        }
        declarations.visit_children(self)
    }
    fn visit_url(&mut self, url: &mut Url<'i>) -> Result<()> {
        url.url = (self.resolve)(&url.url)?.into();
        Ok(())
    }
    fn visit_image(&mut self, image: &mut lightningcss::values::image::Image<'i>) -> Result<()> {
        use lightningcss::values::image::Image;
        match image {
            Image::Url(url) => self.visit_url(url),
            Image::ImageSet(set) => {
                // ImageSetOption skips recursive image types in its derived visitor.
                // Explicitly traverse every option, including quoted URL sources.
                for option in &mut set.options {
                    self.visit_image(&mut option.image)?;
                }
                Ok(())
            }
            _ => image.visit_children(self),
        }
    }
    fn visit_property(&mut self, property: &mut Property<'i>) -> Result<()> {
        match property {
            Property::FontFamily(families) => {
                for family in families {
                    self.family(family)?;
                }
            }
            Property::Font(font) => {
                for family in &mut font.family {
                    self.family(family)?;
                }
            }
            Property::AnimationName(names, _) => {
                for name in names {
                    self.animation(name);
                }
            }
            Property::Animation(animations, _) => {
                for animation in animations {
                    self.animation(&mut animation.name);
                }
            }
            _ => {}
        }
        property.visit_children(self)
    }
    fn visit_function(&mut self, f: &mut Function<'i>) -> Result<()> {
        // Typed image-set() values visit Image -> Url, including quoted sources.
        // Unparsed resource functions cannot bypass URL validation via variables.
        if ![
            "calc",
            "min",
            "max",
            "clamp",
            "rgb",
            "rgba",
            "hsl",
            "hsla",
            "color",
            "color-mix",
            "light-dark",
            "linear-gradient",
            "radial-gradient",
            "conic-gradient",
            "repeating-linear-gradient",
            "repeating-radial-gradient",
            "translate",
            "translatex",
            "translatey",
            "translate3d",
            "scale",
            "scalex",
            "scaley",
            "rotate",
            "rotatex",
            "rotatey",
            "rotatez",
            "skew",
            "matrix",
            "matrix3d",
            "blur",
            "brightness",
            "contrast",
            "drop-shadow",
            "grayscale",
            "hue-rotate",
            "invert",
            "opacity",
            "saturate",
            "sepia",
            "cubic-bezier",
            "steps",
            "format",
        ]
        .contains(&f.name.to_ascii_lowercase().as_str())
        {
            return Err(invalid(format!(
                "不支持未解析的资源/自定义函数 {}()。",
                f.name
            )));
        }
        f.visit_children(self)
    }
    fn visit_token(&mut self, token: &mut TokenOrValue<'i>) -> Result<()> {
        if let TokenOrValue::AnimationName(name) = token {
            self.animation(name);
        }
        if let TokenOrValue::Token(Token::String(s) | Token::Ident(s)) = token {
            let quoted = format!("\"{s}\"");
            if let Some(name) = self
                .fonts
                .get(s.as_ref())
                .or_else(|| self.fonts.get(&quoted))
            {
                *s = name.trim_matches('"').to_owned().into();
            } else if self.animations.contains(s.as_ref()) {
                *s = format!("{}-{s}", self.namespace).into();
            }
        }
        if matches!(token, TokenOrValue::Token(Token::UnquotedUrl(_))) {
            return Err(invalid("未解析的 URL 不受支持。"));
        }
        token.visit_children(self)
    }
}
impl<F> Compiler<'_, F> {
    fn animation(&self, name: &mut lightningcss::properties::animation::AnimationName<'_>) {
        use lightningcss::properties::animation::AnimationName;
        match name {
            AnimationName::Ident(n) => n.0 = format!("{}-{}", self.namespace, n.0).into(),
            AnimationName::String(n) => n.0 = format!("{}-{}", self.namespace, n.0).into(),
            _ => {}
        }
    }
    fn family<'i>(&self, family: &mut FontFamily<'i>) -> Result<()> {
        let key = family
            .to_css_string(PrinterOptions::default())
            .map_err(|e| invalid(e.to_string()))?;
        if let Some(name) = self.fonts.get(&key) {
            *family = FontFamily::parse_string(name)
                .map_err(|e| invalid(format!("字体: {e:?}")))?
                .into_owned();
        }
        Ok(())
    }
}

fn emit(rules: &CssRuleList<'_>, scope: &str) -> Result<String> {
    let mut out = String::new();
    for rule in &rules.0 {
        match rule {
            CssRule::Style(r) => {
                let selectors = r
                    .selectors
                    .0
                    .iter()
                    .map(|s| {
                        scoped(
                            &s.to_css_string(PrinterOptions::default())
                                .map_err(|e| invalid(e.to_string()))?,
                            scope,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                out.push_str(&selectors.join(","));
                out.push('{');
                out.push_str(
                    &r.declarations
                        .to_css_string(PrinterOptions::default())
                        .map_err(|e| invalid(e.to_string()))?,
                );
                out.push('}');
            }
            CssRule::Media(r) => {
                out.push_str(&format!(
                    "@media {}{{{}}}",
                    r.query
                        .to_css_string(PrinterOptions::default())
                        .map_err(|e| invalid(e.to_string()))?,
                    emit(&r.rules, scope)?
                ));
            }
            CssRule::Supports(r) => {
                out.push_str(&format!(
                    "@supports {}{{{}}}",
                    r.condition
                        .to_css_string(PrinterOptions::default())
                        .map_err(|e| invalid(e.to_string()))?,
                    emit(&r.rules, scope)?
                ));
            }
            _ => out.push_str(
                &rule
                    .to_css_string(PrinterOptions::default())
                    .map_err(|e| invalid(e.to_string()))?,
            ),
        }
    }
    Ok(out)
}

pub fn compile(
    sources: &[(String, String)],
    scope: &str,
    namespace: &str,
    mut resolve: impl FnMut(&str, &str) -> Result<String>,
) -> Result<(String, Vec<String>)> {
    let mut sheets = sources
        .iter()
        .map(|(file, css)| {
            StyleSheet::parse(
                css,
                ParserOptions {
                    filename: file.clone(),
                    ..Default::default()
                },
            )
            .map_err(|e| invalid(e.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut fonts = BTreeMap::new();
    let mut animations = BTreeSet::new();
    for sheet in &sheets {
        names(&sheet.rules, &mut fonts, &mut animations, namespace)?;
    }
    let mut out = String::new();
    for (sheet, (file, _)) in sheets.iter_mut().zip(sources) {
        let mut resolver = |url: &str| resolve(file, url);
        sheet
            .visit(&mut Compiler {
                resolve: &mut resolver,
                fonts: &fonts,
                animations: &animations,
                namespace,
            })
            .map_err(|mut e| {
                e.message = format!("{file}: {}", e.message);
                e
            })?;
        out.push_str(&emit(&sheet.rules, scope)?);
    }
    Ok((
        out,
        fonts
            .values()
            .map(|s| s.trim_matches('"').to_owned())
            .collect(),
    ))
}

pub fn inherited_tokens(css: &str, scope: &str) -> Result<String> {
    fn filter(rules: &mut CssRuleList<'_>, scope: &str) -> Result<()> {
        for rule in &mut rules.0 {
            match rule {
                CssRule::Style(r) => {
                    r.selectors.0.retain(|s| {
                        s.to_css_string(PrinterOptions::default())
                            .is_ok_and(|s| s == scope)
                    });
                    r.declarations.declarations.retain(
                        |p| matches!(p, Property::Custom(c) if c.name.as_ref().starts_with("--h-")),
                    );
                    if r.selectors.0.is_empty() || r.declarations.declarations.is_empty() {
                        *rule = CssRule::Ignored;
                    }
                }
                CssRule::Media(r) => filter(&mut r.rules, scope)?,
                CssRule::Supports(r) => filter(&mut r.rules, scope)?,
                CssRule::FontFace(_) => {}
                _ => *rule = CssRule::Ignored,
            }
        }
        Ok(())
    }
    let mut sheet =
        StyleSheet::parse(css, ParserOptions::default()).map_err(|e| invalid(e.to_string()))?;
    filter(&mut sheet.rules, scope)?;
    Ok(sheet
        .to_css(PrinterOptions::default())
        .map_err(|e| invalid(e.to_string()))?
        .code)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(css: &str) -> Result<String> {
        compile(
            &[("theme.css".into(), css.into())],
            "[data-h-theme-root][data-h-session=\"lease\"]",
            "lease",
            |_, url| {
                if url.starts_with("https:") || url.starts_with("data:") {
                    return Err(invalid("external"));
                }
                Ok(format!("/@theme/lease/{url}"))
            },
        )
        .map(|v| v.0)
    }
    #[test]
    fn scope_and_resource_boundaries() {
        for css in [
            ":root + aside { color:red }",
            ":root > div ~ aside {color:red}",
            ":is(:root) + aside {color:red}",
            ":not(:root) {color:red}",
            "body {color:red}",
            ".private {color:red}",
            "#app {color:red}",
            "@import 'https://evil';",
            "button {color:red!important}",
            "button{background:image-set('https://evil' 1x)}",
            "button{--x:image-set('https://evil' 1x);background:var(--x)}",
            "button{background:url(data:image/png,abc)}",
            "[data-h-part=unknown]{color:red}",
        ] {
            assert!(check(css).is_err(), "accepted {css}");
        }
        let css = check(":root{--h-font-size:18px}[data-h-part=card]{padding:24px;background:image-set('scene.png' 1x);border-image:url(frame.png) 12} @media(max-width:900px){button{font-size:20px}} @keyframes glow{to{opacity:.8}}button{animation:glow 1s}").unwrap();
        assert!(css.contains("lease-glow"));
        assert!(css.contains("/@theme/lease/scene.png"));
        assert!(css.contains("[data-h-theme-root][data-h-session="));
    }
    #[test]
    fn font_and_animation_variables_are_namespaced_without_renaming_layout_names() {
        let css = check("@font-face{font-family:'Display';src:url(display.woff2) format('woff2')}@keyframes pulse{to{opacity:.8}}:root{--h-font-body:'Display';--motion:pulse 1s}button{font-family:'Display';animation:var(--motion);grid-area:content}").unwrap();
        assert!(css.contains("lease-font-0"));
        assert!(css.contains("--motion: lease-pulse"));
        assert!(css.contains("grid-area: content"));
        assert!(!css.contains("lease-content"));
    }
}
