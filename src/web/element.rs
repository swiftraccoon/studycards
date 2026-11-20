//! Building HTML documents from a tree of elements.
//!
//! Text and attribute values are escaped when the tree is rendered, so
//! content from a study set cannot become markup.

/// Elements that have no content and no closing tag.
const VOID_ELEMENTS: [&str; 5] = ["br", "hr", "input", "link", "meta"];

/// A piece of a document.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Node {
    /// A nested element.
    Element(Element),
    /// Text, stored unescaped.
    Text(String),
}

/// An HTML or inline SVG element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Element {
    /// The tag name.
    tag: &'static str,
    /// Attribute names and unescaped values; an empty value renders as a
    /// bare attribute name.
    attributes: Vec<(&'static str, String)>,
    /// The content.
    children: Vec<Node>,
}

impl Element {
    /// Creates an element with the given tag name, without attributes or
    /// content.
    pub(super) const fn new(tag: &'static str) -> Self {
        Self {
            tag,
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Sets an attribute, replacing a value given earlier.
    pub(super) fn attr(mut self, name: &'static str, value: &str) -> Self {
        let value = value.to_owned();
        match self
            .attributes
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            Some((_, existing)) => *existing = value,
            None => self.attributes.push((name, value)),
        }
        self
    }

    /// Adds a boolean attribute, such as `autofocus`, when `present`.
    pub(super) fn flag(self, name: &'static str, present: bool) -> Self {
        if present { self.attr(name, "") } else { self }
    }

    /// Sets the `class` attribute.
    pub(super) fn class(self, names: &str) -> Self {
        self.attr("class", names)
    }

    /// Appends text.
    pub(super) fn text(mut self, text: &str) -> Self {
        self.children.push(Node::Text(text.to_owned()));
        self
    }

    /// Appends an element.
    pub(super) fn child(mut self, child: Self) -> Self {
        self.children.push(Node::Element(child));
        self
    }

    /// Appends elements; an `Option` appends one element or none.
    pub(super) fn children(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children
            .extend(children.into_iter().map(Node::Element));
        self
    }

    /// Renders a complete HTML document with this element as its root.
    pub(super) fn to_document(&self) -> String {
        let mut out = String::from("<!DOCTYPE html>\n");
        self.render(&mut out);
        out.push('\n');
        out
    }

    /// Writes the element as markup.
    fn render(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.tag);
        for (name, value) in &self.attributes {
            out.push(' ');
            out.push_str(name);
            if !value.is_empty() {
                out.push_str("=\"");
                escape(value, Place::AttributeValue, out);
                out.push('"');
            }
        }
        out.push('>');
        if VOID_ELEMENTS.contains(&self.tag) {
            return;
        }
        for child in &self.children {
            match child {
                Node::Element(element) => element.render(out),
                Node::Text(text) => escape(text, Place::Content, out),
            }
        }
        out.push_str("</");
        out.push_str(self.tag);
        out.push('>');
    }
}

/// Where text stands in the markup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Place {
    /// Between a start tag and an end tag.
    Content,
    /// Between the double quotes of an attribute value.
    AttributeValue,
}

/// Appends `text` with the characters that are special at `place` replaced.
fn escape(text: &str, place: Place, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if place == Place::AttributeValue => out.push_str("&quot;"),
            '\'' if place == Place::AttributeValue => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Element;

    #[test]
    fn renders_nested_elements_with_attributes() {
        let page = Element::new("html").attr("lang", "en").child(
            Element::new("body")
                .child(Element::new("h1").class("title").text("Hello"))
                .child(
                    Element::new("p")
                        .text("One ")
                        .child(Element::new("em").text("two"))
                        .text(" three"),
                ),
        );
        assert_eq!(
            page.to_document(),
            "<!DOCTYPE html>\n<html lang=\"en\"><body><h1 class=\"title\">Hello</h1>\
             <p>One <em>two</em> three</p></body></html>\n",
            "markup of a small document"
        );
    }

    #[test]
    fn escapes_text_and_attribute_values() {
        let element = Element::new("a")
            .attr("href", "/x?a=1&b=\"2\"")
            .attr("title", "it's <b>")
            .text("<script>alert('x' & \"y\")</script>");
        assert_eq!(
            element.to_document(),
            "<!DOCTYPE html>\n<a href=\"/x?a=1&amp;b=&quot;2&quot;\" title=\"it&#39;s &lt;b&gt;\">\
             &lt;script&gt;alert('x' &amp; \"y\")&lt;/script&gt;</a>\n",
            "special characters are replaced"
        );
    }

    #[test]
    fn setting_an_attribute_again_replaces_it() {
        let element = Element::new("a")
            .class("button")
            .attr("href", "/a")
            .class("button quiet");
        assert_eq!(
            element.to_document(),
            "<!DOCTYPE html>\n<a class=\"button quiet\" href=\"/a\"></a>\n",
            "an element never carries the same attribute twice"
        );
    }

    #[test]
    fn void_elements_have_no_closing_tag() {
        let form = Element::new("form")
            .child(
                Element::new("input")
                    .attr("name", "q")
                    .flag("required", true)
                    .flag("disabled", false),
            )
            .child(Element::new("br"));
        assert_eq!(
            form.to_document(),
            "<!DOCTYPE html>\n<form><input name=\"q\" required><br></form>\n",
            "inputs and breaks are not closed"
        );
    }

    #[test]
    fn children_accepts_lists_and_options() {
        let list = Element::new("ul")
            .children(["a", "b"].map(|item| Element::new("li").text(item)))
            .children(Some(Element::new("li").text("c")))
            .children(None);
        assert_eq!(
            list.to_document(),
            "<!DOCTYPE html>\n<ul><li>a</li><li>b</li><li>c</li></ul>\n",
            "all given children are appended in order"
        );
    }

    #[test]
    fn svg_elements_are_closed() {
        let chart = Element::new("svg")
            .attr("viewBox", "0 0 10 10")
            .child(Element::new("rect").attr("width", "5"));
        assert_eq!(
            chart.to_document(),
            "<!DOCTYPE html>\n<svg viewBox=\"0 0 10 10\"><rect width=\"5\"></rect></svg>\n",
            "shapes get explicit closing tags"
        );
    }
}
