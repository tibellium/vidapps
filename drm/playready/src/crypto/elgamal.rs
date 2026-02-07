/*!
    ElGamal encryption/decryption on ECC P-256.

    Encrypt(message_point, public_key):
      k = random scalar
      point1 = G * k
      point2 = message_point + public_key * k
      return (point1, point2)

    Decrypt(point1, point2, private_key):
      return point2 - point1 * private_key

    The decrypted result is an affine point; its X-coordinate (32 bytes)
    is split into integrity key (first 16) and content key (last 16).
*/
