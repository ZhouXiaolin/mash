#!/usr/bin/env python3
"""
Interactive Prime Number Theorem Verifier
Enter any x value to see π(x) vs x/ln(x)
"""

import math

def is_prime(n: int) -> bool:
    """Check if n is prime (simple trial division)"""
    if n < 2:
        return False
    if n == 2:
        return True
    if n % 2 == 0:
        return False
    
    limit = int(math.sqrt(n)) + 1
    for i in range(3, limit, 2):
        if n % i == 0:
            return False
    return True

def count_primes_up_to(x: int) -> int:
    """Count primes ≤ x (optimized for interactive use)"""
    if x < 2:
        return 0
    
    count = 1 if x >= 2 else 0  # count 2
    
    # Check odd numbers only
    for n in range(3, x + 1, 2):
        if is_prime(n):
            count += 1
    
    return count

def prime_theorem_approx(x: float) -> float:
    """Prime Number Theorem approximation"""
    if x <= 1:
        return 0
    return x / math.log(x)

def li_approx(x: float) -> float:
    """Logarithmic integral approximation (simplified)"""
    if x <= 2:
        return 0
    
    # Simple numerical integration
    n = min(1000, int(x))  # adaptive number of points
    step = (x - 2) / n
    total = 0
    
    for i in range(n):
        t = 2 + i * step
        total += 1 / math.log(t)
    
    return total * step

def interactive_verifier():
    """Interactive verification loop"""
    print("=" * 70)
    print("INTERACTIVE PRIME NUMBER THEOREM VERIFIER")
    print("=" * 70)
    print("Enter x value (or 'q' to quit):")
    print("Note: For x > 10000, computation may take a moment")
    print("=" * 70)
    
    while True:
        try:
            user_input = input("\nEnter x (1-1000000, or 'q'): ").strip()
            
            if user_input.lower() in ['q', 'quit', 'exit']:
                print("\nGoodbye!")
                break
            
            x = int(user_input)
            
            if x <= 0:
                print("Please enter a positive integer.")
                continue
            
            if x > 1000000:
                print("Warning: Large values may be slow. Using optimized method...")
            
            print(f"\nAnalyzing x = {x:,}...")
            
            # Count primes (with progress indicator for large x)
            if x > 10000:
                print("Counting primes...", end='', flush=True)
                pi_x = count_primes_up_to(x)
                print(" Done!")
            else:
                pi_x = count_primes_up_to(x)
            
            # Compute approximations
            theorem = prime_theorem_approx(x)
            li_val = li_approx(x)
            
            # Compute ratios and errors
            if theorem > 0:
                ratio = pi_x / theorem
                error = 100 * (theorem - pi_x) / pi_x
            else:
                ratio = 0
                error = 0
            
            if li_val > 0:
                li_error = 100 * (li_val - pi_x) / pi_x
            else:
                li_error = 0
            
            # Display results
            print("\n" + "=" * 70)
            print("RESULTS:")
            print("=" * 70)
            print(f"π({x:,}) = {pi_x:,} (actual prime count)")
            print(f"x/ln(x) = {theorem:,.1f} (Prime Number Theorem)")
            print(f"li(x)   = {li_val:,.1f} (Logarithmic Integral)")
            print("-" * 70)
            print(f"Ratio π(x)/(x/ln(x)) = {ratio:.6f}")
            print(f"Relative error of x/ln(x): {abs(error):.2f}%")
            print(f"Relative error of li(x): {abs(li_error):.2f}%")
            
            # Interpretation
            print("\nINTERPRETATION:")
            if ratio < 0.9:
                print("  x/ln(x) significantly underestimates π(x)")
            elif ratio > 1.1:
                print("  x/ln(x) significantly overestimates π(x)")
            else:
                print("  x/ln(x) provides a good approximation")
            
            if abs(li_error) < abs(error):
                print("  li(x) is more accurate than x/ln(x)")
            
            # Density information
            if x > 0:
                density = pi_x / x
                asymptotic = 1 / math.log(x) if x > 1 else 0
                print(f"\nPrime density: π(x)/x = {density:.6f}")
                print(f"Asymptotic density: 1/ln(x) = {asymptotic:.6f}")
                print(f"Average prime gap near x ≈ {math.log(x):.1f}")
            
            print("=" * 70)
            
        except ValueError:
            print("Please enter a valid integer or 'q' to quit.")
        except KeyboardInterrupt:
            print("\n\nInterrupted. Goodbye!")
            break
        except Exception as e:
            print(f"Error: {e}")

def main():
    print("Prime Number Theorem: π(x) ~ x/ln(x) as x → ∞")
    print("This interactive tool lets you verify the theorem for any x.")
    print()
    
    # Show some example values
    print("Example values from literature:")
    examples = [
        (10, 4),
        (100, 25),
        (1000, 168),
        (10000, 1229),
        (100000, 9592),
        (1000000, 78498)
    ]
    
    for x, pi_x in examples:
        theorem = prime_theorem_approx(x)
        ratio = pi_x / theorem
        print(f"  x={x:,}: π(x)={pi_x:,}, x/ln(x)={theorem:,.1f}, ratio={ratio:.4f}")
    
    print()
    interactive_verifier()

if __name__ == "__main__":
    main()