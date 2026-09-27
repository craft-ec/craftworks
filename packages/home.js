// HOME: the public page's body. It needs nothing from the node: no connection, no signer, no key.
export function mount(ctx, el) {
  el.innerHTML = `<h2>Welcome</h2>
    <p>This page needs no login. It loaded only the parts it shows: the header, this text and the footer.</p>
    <p><a href="#/me">Who am I?</a> loads the node connection and asks your node's signer.</p>`;
}
