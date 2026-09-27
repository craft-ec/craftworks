// HOME: the public page's body. It needs nothing from the node: no connection, no login, no key.
export function mount(ctx, el) {
  el.innerHTML = `<h2>Welcome</h2>
    <p>This page needs no login. It loaded only the parts it shows: the header, this text and the footer.</p>
    <p>Your <a href="#/account">Account</a> needs a login: it asks you to log in or register, like any website.</p>`;
}
